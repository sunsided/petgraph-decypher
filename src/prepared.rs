//! Prepared queries: parsed and lowered to decypher's HIR once, then executed
//! any number of times.

use std::ops::Range;

use decypher::ast::clause::Return;
use decypher::ast::query::{QueryBody, SinglePartBody, SingleQuery, SingleQueryKind};
use decypher::hir::arena::{Id, Interner};
use decypher::hir::ops::Operation;
use decypher::hir::{BindingId, HirQuery, LowerConfig};

use crate::error::CypherError;
use crate::exec::functions::Function;

/// A Cypher query, parsed and analyzed once.
///
/// ```
/// use petgraph::Graph;
/// use petgraph_decypher::{EdgeData, NodeData, Parameters, Query, Value};
///
/// let mut graph = Graph::<NodeData, EdgeData>::new();
/// let create = Query::new("CREATE (:Person {name: $name})")?;
/// for name in ["Alice", "Bob"] {
///     let params = Parameters::from([("name".to_owned(), Value::from(name))]);
///     create.execute_mut(&mut graph, &params)?;
/// }
///
/// let names = Query::new("MATCH (p:Person) RETURN p.name AS name ORDER BY name")?
///     .execute(&graph, &Parameters::new())?;
/// assert_eq!(names.column("name").collect::<Vec<_>>(), [&Value::from("Alice"), &Value::from("Bob")]);
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Query {
    /// The lowered query and, after it, one unit per `UNION` branch: each
    /// has its own HIR arenas.
    pub(crate) units: Vec<Unit>,
    pub(crate) statements: Vec<Statement>,
    read_only: bool,
}

/// A lowered query with its interned names resolved once.
#[derive(Debug, Clone)]
pub(crate) struct Unit {
    pub hir: HirQuery,
    pub labels: Vec<String>,
    pub rel_types: Vec<String>,
    pub keys: Vec<String>,
    pub parameters: Vec<String>,
    pub functions: Vec<Function>,
}

/// One `;`-separated statement: one branch, or several joined by `UNION`.
#[derive(Debug, Clone)]
pub(crate) struct Statement {
    pub branches: Vec<Branch>,
    /// `UNION` (deduplicating) rather than `UNION ALL`.
    pub distinct: bool,
}

/// A single query: a run of parts of one unit and the columns it returns.
#[derive(Debug, Clone)]
pub(crate) struct Branch {
    pub unit: usize,
    pub parts: Range<usize>,
    pub columns: Vec<String>,
    /// The binding holding each column.
    pub outputs: Vec<BindingId>,
    /// Whether the branch ends with `RETURN`; otherwise it yields no rows.
    pub returns: bool,
}

fn mismatch() -> CypherError {
    CypherError::InvalidQuery("query parts do not match the parsed query".into())
}

impl Unit {
    fn new(hir: HirQuery) -> Result<Self, CypherError> {
        let functions = names(&hir.arenas.functions)
            .iter()
            .map(|name| Function::resolve(name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            labels: names(&hir.arenas.labels),
            rel_types: names(&hir.arenas.relationship_types),
            keys: names(&hir.arenas.property_keys),
            parameters: names(&hir.arenas.parameters),
            functions,
            hir,
        })
    }

    /// The branch of `sq`, made of `parts` of this unit.
    fn branch(
        &self,
        index: usize,
        parts: Range<usize>,
        sq: &SingleQuery,
    ) -> Result<Branch, CypherError> {
        let last = self
            .hir
            .parts
            .get(parts.end.wrapping_sub(1))
            .ok_or_else(mismatch)?;
        let returns = matches!(last.operations.last(), Some(Operation::Return(_)));
        let outputs = if returns {
            self.hir
                .arenas
                .scopes
                .get(last.output_scope)
                .bindings
                .clone()
        } else {
            Vec::new()
        };
        let columns = column_names(final_return(sq), &outputs, &self.hir);
        Ok(Branch {
            unit: index,
            parts,
            columns,
            outputs,
            returns,
        })
    }
}

impl Query {
    /// Parses and analyzes `text`.
    pub fn new(text: &str) -> Result<Self, CypherError> {
        let ast = decypher::parse(text).map_err(|e| CypherError::ParseError(e.to_string()))?;

        let mut catalog = decypher::hir::Catalog::default();
        catalog.set_strict(true);
        let config = LowerConfig { catalog };
        let hir = decypher::hir::lower::lower(&ast, &config)
            .map_err(|d| CypherError::InvalidQuery(d.to_string()))?;

        let mut units = vec![Unit::new(hir)?];
        let mut statements = Vec::new();
        let mut next_part = 0;
        for body in &ast.statements {
            let (single, unions) = match body {
                QueryBody::SingleQuery(sq) => (sq, &[][..]),
                QueryBody::Regular(rq) => (&rq.single_query, &rq.unions[..]),
                QueryBody::Standalone(_) => {
                    return Err(CypherError::Unsupported("procedure calls".into()));
                }
                _ => return Err(CypherError::Unsupported("schema and admin commands".into())),
            };
            if unions.is_empty() {
                let count = match &single.kind {
                    SingleQueryKind::SinglePart(_) => 1,
                    SingleQueryKind::MultiPart(mp) => mp.parts.len() + 1,
                };
                let parts = next_part..next_part + count;
                next_part += count;
                let branch = units[0].branch(0, parts, single)?;
                statements.push(Statement {
                    branches: vec![branch],
                    distinct: false,
                });
                continue;
            }
            // A union is one part holding the branches as queries of their own.
            let Some(Operation::Union(union)) = units[0]
                .hir
                .parts
                .get(next_part)
                .and_then(|p| p.operations.first())
                .cloned()
            else {
                return Err(mismatch());
            };
            next_part += 1;
            let singles = std::iter::once(single).chain(unions.iter().map(|u| &u.single_query));
            if union.branches.len() != unions.len() + 1 {
                return Err(mismatch());
            }
            let mut branches = Vec::new();
            for (sq, hir) in singles.zip(union.branches) {
                let parts = 0..hir.parts.len();
                units.push(Unit::new(hir)?);
                let index = units.len() - 1;
                branches.push(units[index].branch(index, parts, sq)?);
            }
            statements.push(Statement {
                branches,
                distinct: union.distinct,
            });
        }
        if next_part != units[0].hir.parts.len() {
            return Err(mismatch());
        }

        let read_only = !units
            .iter()
            .flat_map(|u| &u.hir.parts)
            .any(|p| p.operations.iter().any(mutates));
        Ok(Self {
            units,
            statements,
            read_only,
        })
    }

    /// Whether executing the query can change the graph.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The names of the parameters the query uses (without `$`).
    pub fn parameters(&self) -> impl Iterator<Item = &str> {
        let mut names: Vec<&str> = self
            .units
            .iter()
            .flat_map(|u| u.parameters.iter().map(String::as_str))
            .collect();
        names.sort_unstable();
        names.dedup();
        names.into_iter()
    }
}

impl std::str::FromStr for Query {
    type Err = CypherError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

fn mutates(op: &Operation) -> bool {
    match op {
        Operation::Create(_)
        | Operation::Merge(_)
        | Operation::Set(_)
        | Operation::Remove(_)
        | Operation::Delete(_) => true,
        Operation::Foreach(f) => f.operations.iter().any(mutates),
        Operation::Union(u) => u
            .branches
            .iter()
            .flat_map(|b| &b.parts)
            .any(|p| p.operations.iter().any(mutates)),
        _ => false,
    }
}

/// Resolves every interned name once; ids are dense, starting at 0.
fn names(interner: &Interner<Id>) -> Vec<String> {
    (0..)
        .map_while(|i| interner.name_of(Id(i)).map(str::to_owned))
        .collect()
}

fn final_return(sq: &SingleQuery) -> Option<&Return> {
    let body = match &sq.kind {
        SingleQueryKind::SinglePart(sp) => &sp.body,
        SingleQueryKind::MultiPart(mp) => &mp.final_part.body,
    };
    match body {
        SinglePartBody::Return(r) => Some(r),
        SinglePartBody::Updating { return_clause, .. } => return_clause.as_ref(),
        _ => None,
    }
}

/// Column names: the binding names of variables passed through by `*`
/// (sorted by the lowering), then each item's alias, variable name, or
/// expression text exactly as written (`count(*)`, `n.name`).
fn column_names(ret: Option<&Return>, outputs: &[BindingId], hir: &HirQuery) -> Vec<String> {
    let items = ret.map_or(&[][..], |r| &r.items[..]);
    let star = outputs.len().saturating_sub(items.len());
    let mut columns: Vec<String> = outputs[..star]
        .iter()
        .map(|&b| hir.arenas.bindings.get(b).name.clone())
        .collect();
    columns.extend(items.iter().map(|item| item.column().0));
    columns.truncate(outputs.len());
    columns
}

impl Query {
    /// Runs the query against `graph`. Fails if the query would modify the
    /// graph; use [`Query::execute_mut`] for those.
    pub fn execute<G: crate::CypherGraph>(
        &self,
        graph: &G,
        params: &crate::Parameters,
    ) -> Result<crate::QueryResult, CypherError> {
        if !self.read_only {
            return Err(CypherError::InvalidQuery(
                "the query modifies the graph; use execute_mut / cypher_mut".into(),
            ));
        }
        crate::exec::Executor::run(self, crate::exec::ReadOnly(graph), params)
    }

    /// Runs the query against `graph`, applying its updates.
    pub fn execute_mut<G: crate::CypherGraphMut>(
        &self,
        graph: &mut G,
        params: &crate::Parameters,
    ) -> Result<crate::QueryResult, CypherError> {
        crate::exec::Executor::run(self, crate::exec::ReadWrite::new(graph), params)
    }
}
