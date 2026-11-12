//! Adapter between the TCK runner and the petgraph-decypher API.
//!
//! This is the only part of the harness that knows the crate's API: it runs
//! setup and test queries, converts results into [`TckValue`]s and takes
//! graph snapshots for side-effect checks.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use petgraph::Graph;
use petgraph_decypher::query::PetgraphCypher;
use petgraph_decypher::{CypherError, CypherValue, EdgeData, NodeData, ResultValue};

use super::value::{Node, Rel, TckValue};

/// Prefix of the identity tags the harness stores in `NodeData::variable` /
/// `EdgeData::variable`. The engine never reads those fields while executing,
/// and they are not visible in query results.
const TAG: &str = "#tck";

/// Why a query did not produce a result.
#[derive(Debug)]
pub enum ExecError {
    /// The engine does not implement a feature the query needs. Never counts
    /// as the error a scenario expects.
    Unsupported(String),
    /// Any other error.
    Failed(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Unsupported(m) => write!(f, "unsupported: {m}"),
            ExecError::Failed(m) => write!(f, "error: {m}"),
        }
    }
}

/// A query result: column names in order, and one map per row.
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<BTreeMap<String, TckValue>>,
}

/// The graph state that side effects are computed from.
#[derive(Default)]
pub struct Snapshot {
    pub nodes: BTreeSet<String>,
    pub rels: BTreeSet<String>,
    pub labels: BTreeSet<String>,
    /// `(element tag, key, canonical value)`.
    pub properties: BTreeSet<(String, String, String)>,
}

#[derive(Default)]
pub struct Engine {
    graph: Graph<NodeData, EdgeData>,
    next_tag: usize,
}

impl Engine {
    /// Runs a setup query (`having executed:` or a named graph script).
    pub fn setup(&mut self, query: &str) -> Result<(), String> {
        self.graph.cypher_mut(query).map_err(|e| e.to_string())
    }

    /// Runs the query under test.
    pub fn execute(
        &mut self,
        query: &str,
        params: &BTreeMap<String, TckValue>,
    ) -> Result<Table, ExecError> {
        let hir = decypher::analyze(query).map_err(|e| ExecError::Failed(e.to_string()))?;
        if is_mutating(&hir) {
            if !params.is_empty() {
                return Err(ExecError::Unsupported(
                    "parameters in a mutating query".into(),
                ));
            }
            self.graph.cypher_mut(query).map_err(exec_error)?;
            return Ok(Table {
                columns: Vec::new(),
                rows: Vec::new(),
            });
        }

        let params = params
            .iter()
            .map(|(k, v)| Ok((k.clone(), to_cypher(v)?)))
            .collect::<Result<HashMap<_, _>, ExecError>>()?;
        let result = self
            .graph
            .cypher_params(query, &params)
            .map_err(exec_error)?;
        let columns = result.columns().to_vec();
        let rows = result
            .map(|row| {
                row.values
                    .into_iter()
                    .map(|(k, v)| (k, from_result(v)))
                    .collect()
            })
            .collect();
        Ok(Table { columns, rows })
    }

    /// Tags elements that have no identity tag yet and records the graph state.
    pub fn snapshot(&mut self) -> Snapshot {
        let mut snap = Snapshot::default();
        for weight in self.graph.node_weights_mut() {
            let tag = tag(&mut weight.variable, &mut self.next_tag);
            snap.labels.extend(weight.labels.iter().cloned());
            record(&mut snap.properties, &tag, &weight.properties);
            snap.nodes.insert(tag);
        }
        for weight in self.graph.edge_weights_mut() {
            let tag = tag(&mut weight.variable, &mut self.next_tag);
            record(&mut snap.properties, &tag, &weight.properties);
            snap.rels.insert(tag);
        }
        snap
    }
}

fn tag(variable: &mut Option<String>, next: &mut usize) -> String {
    if let Some(tag) = variable.as_ref().filter(|v| v.starts_with(TAG)) {
        return tag.clone();
    }
    *next += 1;
    let tag = format!("{TAG}{next}");
    *variable = Some(tag.clone());
    tag
}

fn record(
    out: &mut BTreeSet<(String, String, String)>,
    tag: &str,
    properties: &HashMap<String, CypherValue>,
) {
    for (key, value) in properties {
        if *value != CypherValue::Null {
            out.insert((
                tag.to_owned(),
                key.clone(),
                from_cypher(value).canonical(false),
            ));
        }
    }
}

fn is_mutating(hir: &decypher::hir::HirQuery) -> bool {
    use decypher::hir::ops::Operation;
    hir.parts.iter().flat_map(|p| &p.operations).any(|op| {
        matches!(
            op,
            Operation::Create(_)
                | Operation::Merge(_)
                | Operation::Set(_)
                | Operation::Remove(_)
                | Operation::Delete(_)
        )
    })
}

fn exec_error(e: CypherError) -> ExecError {
    match e {
        CypherError::Unsupported(m) => ExecError::Unsupported(m),
        other => ExecError::Failed(other.to_string()),
    }
}

fn to_cypher(value: &TckValue) -> Result<CypherValue, ExecError> {
    Ok(match value {
        TckValue::Null => CypherValue::Null,
        TckValue::Bool(b) => CypherValue::Boolean(*b),
        TckValue::Int(i) => CypherValue::Integer(*i),
        TckValue::Float(f) => CypherValue::Float(*f),
        TckValue::String(s) => CypherValue::String(s.clone()),
        TckValue::List(items) => {
            CypherValue::List(items.iter().map(to_cypher).collect::<Result<_, _>>()?)
        }
        TckValue::Map(map) => CypherValue::Map(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), to_cypher(v)?)))
                .collect::<Result<_, ExecError>>()?,
        ),
        other => {
            return Err(ExecError::Unsupported(format!(
                "graph element as parameter: {}",
                other.canonical(false)
            )));
        }
    })
}

fn from_cypher(value: &CypherValue) -> TckValue {
    match value {
        CypherValue::Null => TckValue::Null,
        CypherValue::Boolean(b) => TckValue::Bool(*b),
        CypherValue::Integer(i) => TckValue::Int(*i),
        CypherValue::Float(f) => TckValue::Float(*f),
        CypherValue::String(s) => TckValue::String(s.clone()),
        CypherValue::List(items) => TckValue::List(items.iter().map(from_cypher).collect()),
        CypherValue::Map(map) => TckValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), from_cypher(v)))
                .collect(),
        ),
    }
}

fn properties(map: &HashMap<String, CypherValue>) -> BTreeMap<String, TckValue> {
    map.iter()
        .filter(|(_, v)| **v != CypherValue::Null)
        .map(|(k, v)| (k.clone(), from_cypher(v)))
        .collect()
}

fn from_result(value: ResultValue) -> TckValue {
    match value {
        ResultValue::Scalar(v) => from_cypher(&v),
        ResultValue::Node {
            labels,
            properties: props,
        } => TckValue::Node(Node {
            labels,
            properties: properties(&props),
        }),
        ResultValue::Edge {
            rel_type,
            properties: props,
        } => TckValue::Rel(Rel {
            rel_type: rel_type.unwrap_or_default(),
            properties: properties(&props),
        }),
    }
}
