//! Pattern matching by backtracking over one mutable row.

use decypher::hir::pattern::{LabelExpr, PathBinding};
use decypher::hir::{ExprId, GraphPattern, RelationshipDirection, RelationshipLength};
use petgraph::Direction;
use petgraph::graph::{EdgeIndex, NodeIndex};

use super::{Access, Executor, Result, Row};
use crate::element::{CypherEdge, CypherNode};
use crate::error::CypherError;
use crate::graph::CypherGraph;
use crate::value::{Path, Properties, Value};

/// What a pattern relationship matched.
#[derive(Debug, Clone)]
enum Hop {
    One(EdgeIndex),
    /// A variable-length relationship: its edges in pattern order (left to
    /// right).
    Many(Vec<EdgeIndex>),
}

struct Search<'a, 'q, A: Access> {
    ex: &'a Executor<'q, A>,
    pattern: &'a GraphPattern,
    predicates: &'a [ExprId],
    node_props: Vec<Properties>,
    rel_props: Vec<Properties>,
    nodes: Vec<Option<NodeIndex>>,
    hops: Vec<Option<Hop>>,
    /// Edges matched so far: a pattern uses each relationship at most once.
    used: Vec<EdgeIndex>,
    limit: Option<usize>,
    out: Vec<Row>,
}

impl<'q, A: Access> Executor<'q, A> {
    /// All extensions of `row` that match `pattern` and satisfy `predicates`;
    /// at most `limit` of them.
    pub(super) fn match_pattern(
        &self,
        pattern: &GraphPattern,
        row: &Row,
        predicates: &[ExprId],
        limit: Option<usize>,
    ) -> Result<Vec<Row>> {
        let node_props = pattern
            .nodes
            .iter()
            .map(|n| self.properties(n.properties, row))
            .collect::<Result<Vec<_>>>()?;
        let rel_props = pattern
            .relationships
            .iter()
            .map(|r| self.properties(r.properties, row))
            .collect::<Result<Vec<_>>>()?;
        let mut search = Search {
            ex: self,
            pattern,
            predicates,
            node_props,
            rel_props,
            nodes: vec![None; pattern.nodes.len()],
            hops: vec![None; pattern.relationships.len()],
            used: Vec::new(),
            limit,
            out: Vec::new(),
        };
        let mut row = row.clone();
        search.step(&mut row)?;
        Ok(search.out)
    }

    /// Matches of a single-chain pattern with its whole path bound to `path`,
    /// as in `[p = (a)-->(b) | p]`.
    pub(super) fn match_path(
        &self,
        pattern: &GraphPattern,
        path: Option<decypher::hir::BindingId>,
        row: &Row,
    ) -> Result<Vec<Row>> {
        let Some(binding) = path else {
            return self.match_pattern(pattern, row, &[], None);
        };
        let mut pattern = pattern.clone();
        pattern.path_bindings.push(PathBinding {
            binding,
            nodes: (0..pattern.nodes.len()).collect(),
            relationships: (0..pattern.relationships.len()).collect(),
        });
        self.match_pattern(&pattern, row, &[], None)
    }

    /// Whether `node` satisfies a label expression.
    pub(super) fn label_expr_matches(
        &self,
        expr: &LabelExpr,
        has: &dyn Fn(&str) -> bool,
        any: bool,
        for_types: bool,
        row: &Row,
    ) -> Result<bool> {
        Ok(match expr {
            LabelExpr::Name(id) => has(if for_types {
                &self.q.rel_types[id.0]
            } else {
                &self.q.labels[id.0]
            }),
            LabelExpr::Wildcard => any,
            LabelExpr::Dynamic(e) => match self.eval(*e, row)? {
                Value::String(s) => has(&s),
                Value::List(items) => {
                    let mut names = items.iter().map(|v| match v {
                        Value::String(s) => Ok(has(s)),
                        other => Err(CypherError::TypeMismatch(format!(
                            "a dynamic label must be a string, got {}",
                            other.type_name()
                        ))),
                    });
                    if for_types {
                        names.try_fold(false, |acc, r| r.map(|b| acc || b))?
                    } else {
                        names.try_fold(true, |acc, r| r.map(|b| acc && b))?
                    }
                }
                other => {
                    return Err(CypherError::TypeMismatch(format!(
                        "a dynamic label must be a string, got {}",
                        other.type_name()
                    )));
                }
            },
            LabelExpr::Or(a, b) => {
                self.label_expr_matches(a, has, any, for_types, row)?
                    || self.label_expr_matches(b, has, any, for_types, row)?
            }
            LabelExpr::And(a, b) => {
                self.label_expr_matches(a, has, any, for_types, row)?
                    && self.label_expr_matches(b, has, any, for_types, row)?
            }
            LabelExpr::Not(a) => !self.label_expr_matches(a, has, any, for_types, row)?,
        })
    }
}

/// `row` with every variable the pattern would have bound set to `null`, as
/// `OPTIONAL MATCH` does when nothing matches.
pub(super) fn null_unbound(pattern: &GraphPattern, mut row: Row) -> Row {
    let bindings = pattern
        .nodes
        .iter()
        .filter_map(|n| n.binding)
        .chain(pattern.relationships.iter().filter_map(|r| r.binding))
        .chain(pattern.path_bindings.iter().map(|p| p.binding));
    for b in bindings {
        row[b.0].get_or_insert(Value::Null);
    }
    row
}

fn props_match<'v>(expected: &Properties, get: impl Fn(&str) -> Option<&'v Value>) -> bool {
    expected
        .iter()
        .all(|(k, v)| get(k).is_some_and(|actual| actual.cypher_eq(v) == Some(true)))
}

impl<A: Access> Search<'_, '_, A> {
    fn done(&self) -> bool {
        self.limit.is_some_and(|l| self.out.len() >= l)
    }

    fn step(&mut self, row: &mut Row) -> Result<()> {
        if self.done() {
            return Ok(());
        }
        // Expand a relationship next to a matched node, if any.
        let next_rel = (0..self.hops.len()).find(|&i| {
            let r = &self.pattern.relationships[i];
            self.hops[i].is_none()
                && (self.nodes[r.left].is_some() || self.nodes[r.right].is_some())
        });
        if let Some(ri) = next_rel {
            return self.expand(ri, row);
        }
        // Otherwise start a new component, preferring nodes already bound.
        let next_node = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].is_none())
            .min_by_key(|&i| {
                let bound = self.pattern.nodes[i]
                    .binding
                    .is_some_and(|b| row[b.0].is_some());
                !bound
            });
        if let Some(ni) = next_node {
            let candidates: Vec<NodeIndex> = match self.bound_node(ni, row) {
                Some(Some(n)) => vec![n],
                Some(None) => return Ok(()),
                None => self
                    .ex
                    .graph()
                    .node_ids()
                    .filter(|&n| self.ex.node_alive(n))
                    .collect(),
            };
            for n in candidates {
                if self.node_ok(ni, n, row)? {
                    let saved = self.assign_node(ni, n, row);
                    self.step(row)?;
                    self.unassign_node(ni, saved, row);
                    if self.done() {
                        break;
                    }
                }
            }
            return Ok(());
        }
        self.finish(row)
    }

    /// The node a pattern node's variable is already bound to: `None` when
    /// unbound, `Some(None)` when bound to something that is not a node.
    fn bound_node(&self, ni: usize, row: &Row) -> Option<Option<NodeIndex>> {
        let b = self.pattern.nodes[ni].binding?;
        match row[b.0].as_ref()? {
            Value::Node(n) => Some(Some(*n)),
            _ => Some(None),
        }
    }

    fn node_ok(&self, ni: usize, n: NodeIndex, row: &Row) -> Result<bool> {
        if !self.ex.node_alive(n) {
            return Ok(false);
        }
        if let Some(existing) = self.nodes[ni] {
            return Ok(existing == n);
        }
        if let Some(bound) = self.bound_node(ni, row)
            && bound != Some(n)
        {
            return Ok(false);
        }
        let Some(weight) = self.ex.graph().node(n) else {
            return Ok(false);
        };
        let p = &self.pattern.nodes[ni];
        if !p
            .labels
            .iter()
            .all(|l| weight.has_label(&self.ex.q.labels[l.0]))
        {
            return Ok(false);
        }
        if let Some(expr) = &p.label_expression {
            let any = weight.labels().next().is_some();
            if !self
                .ex
                .label_expr_matches(expr, &|l| weight.has_label(l), any, false, row)?
            {
                return Ok(false);
            }
        }
        Ok(props_match(&self.node_props[ni], |k| weight.property(k)))
    }

    fn edge_ok(&self, ri: usize, e: EdgeIndex, row: &Row) -> Result<bool> {
        if !self.ex.edge_alive(e) || self.used.contains(&e) {
            return Ok(false);
        }
        let Some(weight) = self.ex.graph().edge(e) else {
            return Ok(false);
        };
        let p = &self.pattern.relationships[ri];
        if !p.types.is_empty()
            && !p
                .types
                .iter()
                .any(|t| weight.rel_type() == self.ex.q.rel_types[t.0])
        {
            return Ok(false);
        }
        if let Some(expr) = &p.type_expression {
            let ty = weight.rel_type();
            if !self
                .ex
                .label_expr_matches(expr, &|t| t == ty, true, true, row)?
            {
                return Ok(false);
            }
        }
        Ok(props_match(&self.rel_props[ri], |k| weight.property(k)))
    }

    fn assign_node(&mut self, ni: usize, n: NodeIndex, row: &mut Row) -> Option<Option<Value>> {
        self.nodes[ni] = Some(n);
        let b = self.pattern.nodes[ni].binding?;
        Some(row[b.0].replace(Value::Node(n)))
    }

    fn unassign_node(&mut self, ni: usize, saved: Option<Option<Value>>, row: &mut Row) {
        self.nodes[ni] = None;
        if let (Some(b), Some(old)) = (self.pattern.nodes[ni].binding, saved) {
            row[b.0] = old;
        }
    }

    /// The directions to follow from the pattern's `left` (or `right`) node.
    fn directions(dir: RelationshipDirection, from_left: bool) -> &'static [Direction] {
        use RelationshipDirection as D;
        match (dir, from_left) {
            (D::LeftToRight, true) | (D::RightToLeft, false) => &[Direction::Outgoing],
            (D::LeftToRight, false) | (D::RightToLeft, true) => &[Direction::Incoming],
            (D::Undirected | D::Both, _) => &[Direction::Outgoing, Direction::Incoming],
        }
    }

    /// Edges from `node` in `dirs`; a self-loop is reported once.
    fn edges_from(&self, node: NodeIndex, dirs: &[Direction]) -> Vec<(EdgeIndex, NodeIndex)> {
        let graph = self.ex.graph();
        let mut out = Vec::new();
        for &d in dirs {
            for (e, other) in graph.edges_of(node, d) {
                if d == Direction::Incoming && dirs.len() == 2 && other == node {
                    continue;
                }
                out.push((e, other));
            }
        }
        out
    }

    fn expand(&mut self, ri: usize, row: &mut Row) -> Result<()> {
        let rel = &self.pattern.relationships[ri];
        let from_left = self.nodes[rel.left].is_some();
        let (start_pi, end_pi) = if from_left {
            (rel.left, rel.right)
        } else {
            (rel.right, rel.left)
        };
        let start = self.nodes[start_pi].expect("expanded from a matched node");
        let dirs = Self::directions(rel.direction, from_left);
        // A relationship variable bound by an earlier clause.
        let bound_edge = rel.binding.and_then(|b| row[b.0].clone());

        match rel.length {
            RelationshipLength::Single => {
                for (e, other) in self.edges_from(start, dirs) {
                    if let Some(bound) = &bound_edge
                        && *bound != Value::Relationship(e)
                    {
                        continue;
                    }
                    if !self.edge_ok(ri, e, row)? || !self.node_ok(end_pi, other, row)? {
                        continue;
                    }
                    self.with_hop(ri, Hop::One(e), &[e], end_pi, other, row)?;
                    if self.done() {
                        break;
                    }
                }
                Ok(())
            }
            RelationshipLength::Variable { min, max } => {
                if let Some(bound) = bound_edge {
                    return self.follow(ri, start, end_pi, from_left, dirs, bound, row);
                }
                let (min, max) = (min.unwrap_or(1) as usize, max.map(|m| m as usize));
                let mut trail = Vec::new();
                self.walk(
                    ri, start, end_pi, from_left, dirs, min, max, &mut trail, row,
                )
            }
        }
    }

    /// Matches a variable-length relationship bound by an earlier clause: its
    /// edges, in pattern order, must form a walk from `start`.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &mut self,
        ri: usize,
        start: NodeIndex,
        end_pi: usize,
        from_left: bool,
        dirs: &[Direction],
        bound: Value,
        row: &mut Row,
    ) -> Result<()> {
        let Value::List(items) = bound else {
            return Ok(());
        };
        let mut edges = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Relationship(e) => edges.push(e),
                _ => return Ok(()),
            }
        }
        let mut walk = edges.clone();
        if !from_left {
            walk.reverse();
        }
        let mut at = start;
        for &e in &walk {
            let Some((_, next)) = self.edges_from(at, dirs).into_iter().find(|(x, _)| *x == e)
            else {
                return Ok(());
            };
            if !self.edge_ok(ri, e, row)? {
                return Ok(());
            }
            at = next;
        }
        if self.node_ok(end_pi, at, row)? {
            self.with_hop(ri, Hop::Many(edges), &walk, end_pi, at, row)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        ri: usize,
        at: NodeIndex,
        end_pi: usize,
        from_left: bool,
        dirs: &[Direction],
        min: usize,
        max: Option<usize>,
        trail: &mut Vec<EdgeIndex>,
        row: &mut Row,
    ) -> Result<()> {
        if trail.len() >= min && self.node_ok(end_pi, at, row)? {
            let mut edges = trail.clone();
            if !from_left {
                edges.reverse();
            }
            let used = trail.clone();
            self.with_hop(ri, Hop::Many(edges), &used, end_pi, at, row)?;
        }
        if self.done() || max.is_some_and(|m| trail.len() >= m) {
            return Ok(());
        }
        for (e, next) in self.edges_from(at, dirs) {
            if trail.contains(&e) || !self.edge_ok(ri, e, row)? {
                continue;
            }
            trail.push(e);
            self.walk(ri, next, end_pi, from_left, dirs, min, max, trail, row)?;
            trail.pop();
            if self.done() {
                break;
            }
        }
        Ok(())
    }

    /// Binds relationship `ri` and its far node, then continues the search.
    fn with_hop(
        &mut self,
        ri: usize,
        hop: Hop,
        edges: &[EdgeIndex],
        end_pi: usize,
        end: NodeIndex,
        row: &mut Row,
    ) -> Result<()> {
        let rel_value = match &hop {
            Hop::One(e) => Value::Relationship(*e),
            Hop::Many(es) => Value::List(es.iter().map(|&e| Value::Relationship(e)).collect()),
        };
        let binding = self.pattern.relationships[ri].binding;
        let saved_rel = binding.map(|b| row[b.0].replace(rel_value));
        let used_before = self.used.len();
        self.used.extend_from_slice(edges);
        self.hops[ri] = Some(hop);
        let node_was_set = self.nodes[end_pi].is_some();
        let saved_node = if node_was_set {
            None
        } else {
            Some(self.assign_node(end_pi, end, row))
        };

        let result = self.step(row);

        if let Some(saved) = saved_node {
            self.unassign_node(end_pi, saved, row);
        }
        self.hops[ri] = None;
        self.used.truncate(used_before);
        if let (Some(b), Some(old)) = (binding, saved_rel) {
            row[b.0] = old;
        }
        result
    }

    /// Binds path variables, checks `WHERE`, and emits a copy of the row; the
    /// row is only copied once it matched.
    fn finish(&mut self, row: &mut Row) -> Result<()> {
        let mut saved = Vec::with_capacity(self.pattern.path_bindings.len());
        for path in &self.pattern.path_bindings {
            let value = Some(Value::Path(self.path(path)?));
            saved.push((
                path.binding.0,
                std::mem::replace(&mut row[path.binding.0], value),
            ));
        }
        let mut matched = true;
        for &p in self.predicates {
            if !self.ex.is_true(p, row)? {
                matched = false;
                break;
            }
        }
        if matched {
            self.out.push(row.clone());
        }
        for (slot, old) in saved.into_iter().rev() {
            row[slot] = old;
        }
        Ok(())
    }

    fn path(&self, binding: &PathBinding) -> Result<Path> {
        let graph = self.ex.graph();
        let first = binding
            .nodes
            .first()
            .and_then(|&i| self.nodes[i])
            .ok_or_else(|| CypherError::RuntimeError("empty path".into()))?;
        let mut path = Path {
            nodes: vec![first],
            relationships: Vec::new(),
        };
        let mut at = first;
        for &ri in &binding.relationships {
            let edges: &[EdgeIndex] = match self.hops[ri].as_ref() {
                Some(Hop::One(e)) => std::slice::from_ref(e),
                Some(Hop::Many(es)) => es,
                None => return Err(CypherError::RuntimeError("unmatched path step".into())),
            };
            for &e in edges {
                let (s, t) = graph
                    .endpoints(e)
                    .ok_or_else(|| CypherError::RuntimeError("dangling relationship".into()))?;
                at = if s == at { t } else { s };
                path.relationships.push(e);
                path.nodes.push(at);
            }
        }
        Ok(path)
    }
}
