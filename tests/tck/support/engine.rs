//! Adapter between the TCK runner and the petgraph-decypher API.
//!
//! This is the only part of the harness that knows the crate's API: it runs
//! setup and test queries, converts results into [`TckValue`]s and takes
//! graph snapshots for side-effect checks. The graph is a `StableGraph`, so
//! petgraph indices identify elements across deletions.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::stable_graph::StableGraph;
use petgraph_decypher::{CypherError, EdgeData, NodeData, Parameters, Query, Value};

use super::value::{self, Node, Rel, TckValue};

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
    /// `(element, key, canonical value)`.
    pub properties: BTreeSet<(String, String, String)>,
}

type TckGraph = StableGraph<NodeData, EdgeData>;

#[derive(Default)]
pub struct Engine {
    graph: TckGraph,
}

impl Engine {
    /// Runs a setup query (`having executed:` or a named graph script).
    pub fn setup(&mut self, query: &str) -> Result<(), String> {
        Query::new(query)
            .and_then(|q| q.execute_mut(&mut self.graph, &Parameters::new()))
            .map(drop)
            .map_err(|e| e.to_string())
    }

    /// Runs the query under test.
    pub fn execute(
        &mut self,
        query: &str,
        params: &BTreeMap<String, TckValue>,
    ) -> Result<Table, ExecError> {
        let params = params
            .iter()
            .map(|(k, v)| Ok((k.clone(), to_value(v)?)))
            .collect::<Result<Parameters, ExecError>>()?;
        let query = Query::new(query).map_err(exec_error)?;
        let result = if query.is_read_only() {
            query.execute(&self.graph, &params)
        } else {
            query.execute_mut(&mut self.graph, &params)
        }
        .map_err(exec_error)?;
        let columns = result.columns().to_vec();
        let rows = result
            .rows()
            .map(|row| {
                row.iter()
                    .map(|(k, v)| (k.to_owned(), self.convert(v)))
                    .collect()
            })
            .collect();
        Ok(Table { columns, rows })
    }

    /// Records the graph state.
    pub fn snapshot(&mut self) -> Snapshot {
        let mut snap = Snapshot::default();
        for n in self.graph.node_indices() {
            let id = format!("n{}", n.index());
            let weight = &self.graph[n];
            snap.labels.extend(weight.labels.iter().cloned());
            record(&mut snap.properties, &id, &weight.properties);
            snap.nodes.insert(id);
        }
        for e in self.graph.edge_indices() {
            let id = format!("r{}", e.index());
            record(&mut snap.properties, &id, &self.graph[e].properties);
            snap.rels.insert(id);
        }
        snap
    }

    fn convert(&self, value: &Value) -> TckValue {
        match value {
            Value::Null => TckValue::Null,
            Value::Boolean(b) => TckValue::Bool(*b),
            Value::Integer(i) => TckValue::Int(*i),
            Value::Float(f) => TckValue::Float(*f),
            Value::String(s) => TckValue::String(s.clone()),
            Value::List(items) => TckValue::List(items.iter().map(|v| self.convert(v)).collect()),
            Value::Map(map) => TckValue::Map(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.convert(v)))
                    .collect(),
            ),
            Value::Node(n) => TckValue::Node(self.node(*n)),
            Value::Relationship(e) => TckValue::Rel(self.rel(*e)),
            Value::Path(p) => {
                let mut hops = Vec::new();
                for (i, &e) in p.relationships.iter().enumerate() {
                    let forward = self
                        .graph
                        .edge_endpoints(e)
                        .is_some_and(|(s, _)| s == p.nodes[i]);
                    hops.push((self.rel(e), forward, self.node(p.nodes[i + 1])));
                }
                TckValue::Path(value::Path {
                    start: self.node(p.nodes[0]),
                    hops,
                })
            }
        }
    }

    fn node(&self, n: petgraph::graph::NodeIndex) -> Node {
        self.graph
            .node_weight(n)
            .map(|w| Node {
                labels: w.labels.clone(),
                properties: self.props(&w.properties),
            })
            .unwrap_or_default()
    }

    fn rel(&self, e: petgraph::graph::EdgeIndex) -> Rel {
        self.graph
            .edge_weight(e)
            .map(|w| Rel {
                rel_type: w.rel_type.clone(),
                properties: self.props(&w.properties),
            })
            .unwrap_or_default()
    }

    fn props(&self, map: &BTreeMap<String, Value>) -> BTreeMap<String, TckValue> {
        map.iter()
            .map(|(k, v)| (k.clone(), self.convert(v)))
            .collect()
    }
}

fn record(
    out: &mut BTreeSet<(String, String, String)>,
    id: &str,
    properties: &BTreeMap<String, Value>,
) {
    for (key, value) in properties {
        out.insert((id.to_owned(), key.clone(), value.to_string()));
    }
}

fn exec_error(e: CypherError) -> ExecError {
    match e {
        CypherError::Unsupported(m) => ExecError::Unsupported(m),
        other => ExecError::Failed(other.to_string()),
    }
}

fn to_value(value: &TckValue) -> Result<Value, ExecError> {
    Ok(match value {
        TckValue::Null => Value::Null,
        TckValue::Bool(b) => Value::Boolean(*b),
        TckValue::Int(i) => Value::Integer(*i),
        TckValue::Float(f) => Value::Float(*f),
        TckValue::String(s) => Value::String(s.clone()),
        TckValue::List(items) => Value::List(items.iter().map(to_value).collect::<Result<_, _>>()?),
        TckValue::Map(map) => Value::Map(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), to_value(v)?)))
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
