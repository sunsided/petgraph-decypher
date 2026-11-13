//! Executes a prepared [`Query`] directly on decypher's HIR.
//!
//! Rows are vectors of slots indexed by HIR [`BindingId`]: `None` is an
//! unbound variable. Each operation of a query part maps the current rows to
//! new ones.

mod eval;
pub(crate) mod functions;
mod mutate;
mod pattern;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashSet};

use decypher::hir::ExprId;
use decypher::hir::ops::{Operation, ProjectOp, SortDirection};
use petgraph::Direction;
use petgraph::graph::{EdgeIndex, NodeIndex};

use crate::error::CypherError;
use crate::graph::CypherGraph;
use crate::prepared::{Branch, Query, Unit};
use crate::result::QueryResult;
use crate::value::{Properties, Value};
use functions::{Accumulator, Function};

pub(crate) use mutate::{Access, ReadOnly, ReadWrite};

pub(crate) type Row = Vec<Option<Value>>;
type Result<T> = std::result::Result<T, CypherError>;

/// Query parameters by name (without the leading `$`).
pub type Parameters = std::collections::HashMap<String, Value>;

pub(crate) struct Executor<'q, A: Access> {
    query: &'q Query,
    /// The unit (HIR arenas and names) of the branch being executed.
    q: &'q Unit,
    access: A,
    /// Parameter values per unit, indexed by its `ParameterId`s.
    unit_params: Vec<Vec<Value>>,
    /// The index of `q` in the query's units.
    unit: usize,
    /// Row width of `q`: its number of bindings.
    slots: usize,
    deleted_nodes: HashSet<NodeIndex>,
    deleted_edges: HashSet<EdgeIndex>,
    /// When the query started, in milliseconds since the Unix epoch:
    /// `timestamp()` returns the same value for the whole query.
    started_ms: i64,
}

impl<'q, A: Access> Executor<'q, A> {
    /// Runs every statement; the result is the last statement's.
    pub(crate) fn run(query: &'q Query, access: A, params: &Parameters) -> Result<QueryResult> {
        let unit_params = query
            .units
            .iter()
            .map(|unit| {
                unit.parameters
                    .iter()
                    .map(|name| {
                        params.get(name).cloned().ok_or_else(|| {
                            CypherError::InvalidQuery(format!("missing parameter ${name}"))
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let mut ex = Executor {
            query,
            q: &query.units[0],
            access,
            unit_params,
            unit: 0,
            slots: 0,
            deleted_nodes: HashSet::new(),
            deleted_edges: HashSet::new(),
            started_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
        };
        ex.enter_unit(0);
        match ex.run_statements() {
            Ok(result) => Ok(result),
            Err(e) => {
                ex.access.rollback();
                Err(e)
            }
        }
    }

    /// Makes `unit` the one expressions, names and parameters resolve against.
    fn enter_unit(&mut self, unit: usize) {
        self.unit = unit;
        self.q = &self.query.units[unit];
        self.slots = self.q.hir.arenas.bindings.iter().count();
    }

    /// The value of a parameter of the current unit.
    pub(super) fn param(&self, id: decypher::hir::arena::ParameterId) -> &Value {
        &self.unit_params[self.unit][id.0]
    }

    fn run_statements(&mut self) -> Result<QueryResult> {
        let mut result = QueryResult::default();
        for statement in &self.query.statements {
            let mut rows = Vec::new();
            for branch in &statement.branches {
                rows.extend(self.run_branch(branch)?);
            }
            if statement.distinct {
                dedup(&mut rows, Clone::clone);
            }
            let columns = statement
                .branches
                .first()
                .map(|b| b.columns.clone())
                .unwrap_or_default();
            result = QueryResult::new(columns, rows);
        }
        self.apply_deletes()?;
        Ok(result)
    }

    fn run_branch(&mut self, branch: &Branch) -> Result<Vec<Vec<Value>>> {
        self.enter_unit(branch.unit);
        let mut rows = vec![vec![None; self.slots]];
        for part in &self.q.hir.parts[branch.parts.clone()] {
            for op in &part.operations {
                rows = self.op(op, rows)?;
            }
        }
        if !branch.returns {
            return Ok(Vec::new());
        }
        Ok(rows
            .into_iter()
            .map(|row| {
                branch
                    .outputs
                    .iter()
                    .map(|b| row[b.0].clone().unwrap_or_default())
                    .collect()
            })
            .collect())
    }

    fn op(&mut self, op: &'q Operation, rows: Vec<Row>) -> Result<Vec<Row>> {
        match op {
            Operation::Match(m) => {
                let mut out = Vec::new();
                for row in &rows {
                    out.extend(self.match_pattern(&m.pattern, row, &m.predicates, None)?);
                }
                Ok(out)
            }
            Operation::OptionalMatch(m) => {
                let mut out = Vec::new();
                for row in rows {
                    let matches = self.match_pattern(&m.pattern, &row, &m.predicates, None)?;
                    if matches.is_empty() {
                        out.push(pattern::null_unbound(&m.pattern, row));
                    } else {
                        out.extend(matches);
                    }
                }
                Ok(out)
            }
            Operation::Unwind(u) => {
                let mut out = Vec::new();
                for row in rows {
                    let items = match self.eval(u.expression, &row)? {
                        Value::Null => continue,
                        Value::List(items) => items,
                        other => vec![other],
                    };
                    for item in items {
                        let mut row = row.clone();
                        row[u.variable.0] = Some(item);
                        out.push(row);
                    }
                }
                Ok(out)
            }
            Operation::Filter(f) => {
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    if self.is_true(f.predicate, &row)? {
                        out.push(row);
                    }
                }
                Ok(out)
            }
            Operation::Project(p) => self.project(p, rows),
            Operation::Aggregate(a) => self.aggregate(a, rows),
            Operation::Sort(s) => {
                let mut keyed = Vec::with_capacity(rows.len());
                for row in rows {
                    let keys = s
                        .items
                        .iter()
                        .map(|item| self.eval(item.expression, &row))
                        .collect::<Result<Vec<_>>>()?;
                    keyed.push((keys, row));
                }
                keyed.sort_by(|(a, _), (b, _)| {
                    s.items
                        .iter()
                        .zip(a.iter().zip(b))
                        .map(|(item, (x, y))| match item.direction {
                            SortDirection::Ascending => x.order(y),
                            SortDirection::Descending => y.order(x),
                        })
                        .find(|o| o.is_ne())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                Ok(keyed.into_iter().map(|(_, row)| row).collect())
            }
            Operation::Skip(s) => {
                let n = self.count(s.count, "SKIP")?;
                Ok(rows.into_iter().skip(n).collect())
            }
            Operation::Limit(l) => {
                let n = self.count(l.count, "LIMIT")?;
                Ok(rows.into_iter().take(n).collect())
            }
            Operation::Create(c) => {
                let mut out = Vec::with_capacity(rows.len());
                for mut row in rows {
                    self.create_pattern(&c.pattern, &mut row)?;
                    out.push(row);
                }
                Ok(out)
            }
            Operation::Merge(m) => {
                let mut out = Vec::new();
                for row in rows {
                    out.extend(self.merge(m, row)?);
                }
                Ok(out)
            }
            Operation::Set(s) => {
                for row in &rows {
                    self.set_items(&s.items, row)?;
                }
                Ok(rows)
            }
            Operation::Remove(r) => {
                for row in &rows {
                    self.remove_items(&r.items, row)?;
                }
                Ok(rows)
            }
            Operation::Delete(d) => {
                for row in &rows {
                    for &target in &d.targets {
                        let value = self.eval(target, row)?;
                        self.delete(value, d.detach)?;
                    }
                }
                Ok(rows)
            }
            Operation::Foreach(f) => {
                for row in &rows {
                    let items = match self.eval(f.list, row)? {
                        Value::Null => continue,
                        Value::List(items) => items,
                        other => {
                            return Err(CypherError::TypeMismatch(format!(
                                "FOREACH expects a list, got {}",
                                other.type_name()
                            )));
                        }
                    };
                    for item in items {
                        let mut inner = row.clone();
                        inner[f.variable.0] = Some(item);
                        let mut inner_rows = vec![inner];
                        for op in &f.operations {
                            inner_rows = self.op(op, inner_rows)?;
                        }
                    }
                }
                Ok(rows)
            }
            Operation::Return(_) | Operation::Finish => Ok(rows),
            Operation::CallSubquery(_) => Err(CypherError::Unsupported("CALL subqueries".into())),
            Operation::CallProcedure(_) => Err(CypherError::Unsupported("procedure calls".into())),
            Operation::LoadCsv(_) => Err(CypherError::Unsupported("LOAD CSV".into())),
            Operation::Union(_) => Err(CypherError::Unsupported("nested UNION".into())),
        }
    }

    /// `WITH` / `RETURN` items. Earlier bindings stay in the row: `ORDER BY`
    /// and `WHERE` after a projection may still read them, and later clauses
    /// cannot name them.
    fn project(&self, p: &ProjectOp, rows: Vec<Row>) -> Result<Vec<Row>> {
        let mut out = Vec::with_capacity(rows.len());
        for mut row in rows {
            let values = p
                .items
                .iter()
                .map(|item| self.eval(item.expression, &row))
                .collect::<Result<Vec<_>>>()?;
            for (item, value) in p.items.iter().zip(values) {
                row[item.alias.0] = Some(value);
            }
            out.push(row);
        }
        if p.distinct {
            dedup(&mut out, |row| {
                p.items
                    .iter()
                    .map(|i| row[i.alias.0].clone().unwrap_or_default())
                    .collect()
            });
        }
        Ok(out)
    }

    fn aggregate(&self, a: &decypher::hir::ops::AggregateOp, rows: Vec<Row>) -> Result<Vec<Row>> {
        let mut groups: Vec<(Vec<Value>, Vec<Accumulator>)> = Vec::new();
        let mut index: BTreeMap<Ordered, usize> = BTreeMap::new();
        let new_group = |q: &Unit| {
            a.aggregates
                .iter()
                .map(|item| match q.functions[item.function.0] {
                    Function::Aggregate(f) => {
                        f.check_arity(item.args.len())?;
                        Ok(Accumulator::new(f, item.distinct))
                    }
                    Function::Scalar(_) => Err(CypherError::InvalidQuery(
                        "aggregation uses a non-aggregate function".into(),
                    )),
                })
                .collect::<Result<Vec<_>>>()
        };
        for row in &rows {
            let key = a
                .grouping_keys
                .iter()
                .map(|k| self.eval(k.expression, row))
                .collect::<Result<Vec<_>>>()?;
            let group = match index.entry(Ordered(key)) {
                Entry::Occupied(e) => *e.get(),
                Entry::Vacant(e) => {
                    groups.push((e.key().0.clone(), new_group(self.q)?));
                    *e.insert(groups.len() - 1)
                }
            };
            for (item, acc) in a.aggregates.iter().zip(&mut groups[group].1) {
                if item.args.is_empty() {
                    acc.add(None);
                } else {
                    let args = item
                        .args
                        .iter()
                        .map(|&e| self.eval(e, row))
                        .collect::<Result<Vec<_>>>()?;
                    acc.add(Some(args));
                }
            }
        }
        if groups.is_empty() && a.grouping_keys.is_empty() {
            groups.push((Vec::new(), new_group(self.q)?));
        }
        let mut out = Vec::with_capacity(groups.len());
        for (key, accs) in groups {
            let mut row = vec![None; self.slots];
            for (k, value) in a.grouping_keys.iter().zip(key) {
                row[k.alias.0] = Some(value);
            }
            for (item, acc) in a.aggregates.iter().zip(accs) {
                row[item.alias.0] = Some(acc.finish()?);
            }
            out.push(row);
        }
        Ok(out)
    }

    /// The row count of `SKIP` / `LIMIT`: a non-negative integer.
    fn count(&self, expr: ExprId, clause: &str) -> Result<usize> {
        match self.eval(expr, &vec![None; self.slots])? {
            Value::Integer(n) if n >= 0 => Ok(n as usize),
            other => Err(CypherError::InvalidQuery(format!(
                "{clause} expects a non-negative integer, got {other}"
            ))),
        }
    }

    /// Whether a predicate holds: `null` does not, a non-boolean is an error.
    fn is_true(&self, expr: ExprId, row: &Row) -> Result<bool> {
        match self.eval(expr, row)? {
            Value::Boolean(b) => Ok(b),
            Value::Null => Ok(false),
            other => Err(CypherError::TypeMismatch(format!(
                "a predicate must be a boolean, got {}",
                other.type_name()
            ))),
        }
    }

    fn graph(&self) -> &A::G {
        self.access.graph()
    }

    pub(crate) fn node_alive(&self, node: NodeIndex) -> bool {
        !self.deleted_nodes.contains(&node)
    }

    pub(crate) fn edge_alive(&self, edge: EdgeIndex) -> bool {
        !self.deleted_edges.contains(&edge)
    }

    fn delete(&mut self, value: Value, detach: bool) -> Result<()> {
        match value {
            Value::Null => Ok(()),
            Value::Node(n) => {
                if detach {
                    let edges: Vec<EdgeIndex> = [Direction::Outgoing, Direction::Incoming]
                        .into_iter()
                        .flat_map(|d| self.graph().edges_of(n, d).map(|(e, _)| e))
                        .collect();
                    self.deleted_edges.extend(edges);
                }
                self.deleted_nodes.insert(n);
                Ok(())
            }
            Value::Relationship(e) => {
                self.deleted_edges.insert(e);
                Ok(())
            }
            Value::Path(p) => {
                self.deleted_edges.extend(p.relationships);
                for n in p.nodes {
                    self.delete(Value::Node(n), detach)?;
                }
                Ok(())
            }
            other => Err(CypherError::TypeMismatch(format!(
                "DELETE expects a node, relationship or path, got {}",
                other.type_name()
            ))),
        }
    }

    /// Removes deleted elements once the query has run, refusing to leave
    /// relationships without an end node.
    fn apply_deletes(&mut self) -> Result<()> {
        if self.deleted_nodes.is_empty() && self.deleted_edges.is_empty() {
            return Ok(());
        }
        for &n in &self.deleted_nodes {
            let connected = [Direction::Outgoing, Direction::Incoming]
                .into_iter()
                .flat_map(|d| self.graph().edges_of(n, d))
                .any(|(e, _)| !self.deleted_edges.contains(&e));
            if connected {
                return Err(CypherError::RuntimeError(
                    "cannot delete a node that still has relationships; use DETACH DELETE".into(),
                ));
            }
        }
        let nodes = self.deleted_nodes.drain().collect();
        let edges = self.deleted_edges.drain().collect();
        self.access.remove(nodes, edges)
    }

    /// Evaluates a property map expression of a pattern.
    fn properties(&self, expr: Option<ExprId>, row: &Row) -> Result<Properties> {
        let Some(expr) = expr else {
            return Ok(Properties::new());
        };
        match self.eval(expr, row)? {
            Value::Map(map) => Ok(map),
            Value::Null => Ok(Properties::new()),
            other => Err(CypherError::TypeMismatch(format!(
                "expected a property map, got {}",
                other.type_name()
            ))),
        }
    }
}

/// Values ordered by [`Value::order`], for grouping and `DISTINCT`.
struct Ordered(Vec<Value>);

impl PartialEq for Ordered {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Ordered {}

impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| a.order(b))
            .find(|o| o.is_ne())
            .unwrap_or_else(|| self.0.len().cmp(&other.0.len()))
    }
}

/// Drops rows whose key is equivalent to an earlier row's, keeping the first.
fn dedup<T>(rows: &mut Vec<T>, key: impl Fn(&T) -> Vec<Value>) {
    let mut seen = std::collections::BTreeSet::new();
    rows.retain(|row| seen.insert(Ordered(key(row))));
}
