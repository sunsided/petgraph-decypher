//! Query execution engine for running Cypher queries against a petgraph.

use std::collections::HashMap;

use itertools::Itertools;
use petgraph::graph::{EdgeIndex, EdgeReference, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Graph;

use crate::ast::*;
use crate::error::CypherError;
use crate::{CypherEdge, CypherNode, CypherProperties, EdgeData, NodeData};

/// Strategy to use when matching patterns against the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchStrategy {
    /// DFS backtracking (Neo4j-accurate semantics). Default strategy.
    #[default]
    Backtrack,
    /// Uses petgraph-native algorithms (BFS/DFS, faster but less accurate).
    /// Currently delegates to Backtrack; a distinct optimized implementation is planned.
    Fast,
}

/// A value that can appear in a query result row.
#[derive(Debug, Clone, PartialEq)]
pub enum ResultValue {
    /// A scalar property value.
    Scalar(CypherValue),
    /// A matched node with its labels and properties.
    Node {
        labels: Vec<String>,
        properties: HashMap<String, CypherValue>,
    },
    /// A matched edge with its type and properties.
    Edge {
        rel_type: Option<String>,
        properties: HashMap<String, CypherValue>,
    },
}

/// A single row of query results.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub values: HashMap<String, ResultValue>,
}

/// A bound value in the query execution context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BoundValue {
    Node(NodeIndex),
    Edge(EdgeIndex),
}

/// A set of variable bindings from pattern matching.
type Bindings = HashMap<String, BoundValue>;

/// An iterator over query result rows.
///
/// The MATCH phase is executed eagerly to collect all bindings.
/// The RETURN phase is executed lazily, projecting one row at a time.
pub struct QueryResult<'a> {
    columns: Vec<String>,
    bindings: std::vec::IntoIter<Bindings>,
    graph: &'a Graph<NodeData, EdgeData>,
    items: Vec<ReturnItem>,
}

impl<'a> QueryResult<'a> {
    fn new(
        columns: Vec<String>,
        bindings: Vec<Bindings>,
        graph: &'a Graph<NodeData, EdgeData>,
        items: Vec<ReturnItem>,
    ) -> Self {
        Self {
            columns,
            bindings: bindings.into_iter(),
            graph,
            items,
        }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    fn project_row(&self, bindings: &Bindings) -> Row {
        let mut values = HashMap::new();

        for item in &self.items {
            let key = if let Some(ref alias) = item.alias {
                alias.clone()
            } else {
                match &item.expression {
                    Expression::Variable(v) => v.clone(),
                    Expression::Property(v, p) => format!("{v}.{p}"),
                    Expression::All => "*".to_string(),
                }
            };

            if matches!(item.expression, Expression::All) {
                for (var, bound) in bindings {
                    match bound {
                        BoundValue::Node(idx) => {
                            let data = &self.graph[*idx];
                            values.insert(
                                var.clone(),
                                ResultValue::Node {
                                    labels: data.labels(),
                                    properties: data.properties(),
                                },
                            );
                        }
                        BoundValue::Edge(idx) => {
                            let data = &self.graph[*idx];
                            values.insert(
                                var.clone(),
                                ResultValue::Edge {
                                    rel_type: data.rel_type().map(str::to_string),
                                    properties: data.properties(),
                                },
                            );
                        }
                    }
                }
                continue;
            }

            let result_value = match &item.expression {
                Expression::Variable(var) => {
                    if let Some(&bound) = bindings.get(var) {
                        match bound {
                            BoundValue::Node(idx) => {
                                let data = &self.graph[idx];
                                ResultValue::Node {
                                    labels: data.labels(),
                                    properties: data.properties(),
                                }
                            }
                            BoundValue::Edge(idx) => {
                                let data = &self.graph[idx];
                                ResultValue::Edge {
                                    rel_type: data.rel_type().map(str::to_string),
                                    properties: data.properties(),
                                }
                            }
                        }
                    } else {
                        ResultValue::Scalar(CypherValue::Null)
                    }
                }
                Expression::Property(var, prop) => {
                    if let Some(&bound) = bindings.get(var) {
                        match bound {
                            BoundValue::Node(idx) => {
                                let data = &self.graph[idx];
                                data.get(prop)
                                    .cloned()
                                    .map(ResultValue::Scalar)
                                    .unwrap_or(ResultValue::Scalar(CypherValue::Null))
                            }
                            BoundValue::Edge(idx) => {
                                let data = &self.graph[idx];
                                data.get(prop)
                                    .cloned()
                                    .map(ResultValue::Scalar)
                                    .unwrap_or(ResultValue::Scalar(CypherValue::Null))
                            }
                        }
                    } else {
                        ResultValue::Scalar(CypherValue::Null)
                    }
                }
                Expression::All => unreachable!(),
            };

            values.insert(key, result_value);
        }

        Row { values }
    }
}

impl<'a> Iterator for QueryResult<'a> {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        let bindings = self.bindings.next()?;
        Some(self.project_row(&bindings))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.bindings.size_hint()
    }
}

/// Extension trait for executing Cypher queries on a [`Graph<NodeData, EdgeData>`].
pub trait PetgraphCypher {
    /// Execute a read-only Cypher query and stream the results.
    ///
    /// Supports `MATCH`, `WHERE`, and `RETURN` clauses.
    /// Returns an error if mutation clauses (`CREATE`, `MERGE`, `DELETE`) are present.
    ///
    /// Uses the default `Backtrack` match strategy.
    fn cypher(&self, query: &str) -> Result<QueryResult<'_>, CypherError>;

    /// Execute a mutating Cypher query.
    ///
    /// Supports `MATCH`, `WHERE`, `CREATE`, `MERGE`, and `DELETE` clauses.
    /// Returns an error if a `RETURN` clause is present — use [`Self::cypher`] for reads.
    fn cypher_mut(&mut self, query: &str) -> Result<(), CypherError>;

    /// Execute a read-only Cypher query with a specific match strategy.
    fn cypher_with_strategy(
        &self,
        query: &str,
        strategy: MatchStrategy,
    ) -> Result<QueryResult<'_>, CypherError>;
}

impl PetgraphCypher for Graph<NodeData, EdgeData> {
    fn cypher(&self, query: &str) -> Result<QueryResult<'_>, CypherError> {
        self.cypher_with_strategy(query, MatchStrategy::default())
    }

    fn cypher_mut(&mut self, query: &str) -> Result<(), CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_mutation_query(&ast)?;
        MutQueryExecutor::new(self).execute(ast)
    }

    fn cypher_with_strategy(
        &self,
        query: &str,
        strategy: MatchStrategy,
    ) -> Result<QueryResult<'_>, CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_read_query(&ast)?;
        ReadQueryExecutor::new(self, strategy).execute(ast)
    }
}

/// Validate that a query only contains read clauses.
fn validate_read_query(query: &CypherQuery) -> Result<(), CypherError> {
    for clause in &query.clauses {
        match clause {
            Clause::Create { .. } => {
                return Err(CypherError::Unsupported(
                    "CREATE not allowed in cypher(); use cypher_mut()".into(),
                ));
            }
            Clause::Merge { .. } => {
                return Err(CypherError::Unsupported(
                    "MERGE not allowed in cypher(); use cypher_mut()".into(),
                ));
            }
            Clause::Delete { .. } => {
                return Err(CypherError::Unsupported(
                    "DELETE not allowed in cypher(); use cypher_mut()".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Validate that a mutation query does not contain read clauses.
fn validate_mutation_query(query: &CypherQuery) -> Result<(), CypherError> {
    for clause in &query.clauses {
        if matches!(clause, Clause::Return { .. }) {
            return Err(CypherError::Unsupported(
                "RETURN not allowed in cypher_mut(); use cypher() for reads".into(),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared pattern matcher
// ---------------------------------------------------------------------------

/// Shared matcher for pattern matching against a graph. Used by both the read
/// and mutation executors to avoid code duplication.
struct PatternMatcher<'a> {
    graph: &'a Graph<NodeData, EdgeData>,
    strategy: MatchStrategy,
}

impl<'a> PatternMatcher<'a> {
    fn new(graph: &'a Graph<NodeData, EdgeData>, strategy: MatchStrategy) -> Self {
        Self { graph, strategy }
    }

    fn match_patterns(&self, patterns: &[PathPattern], base_bindings: &Bindings) -> Vec<Bindings> {
        if patterns.is_empty() {
            return vec![base_bindings.clone()];
        }

        let per_pattern: Vec<Vec<Bindings>> = patterns
            .iter()
            .map(|p| self.match_single_path(p, base_bindings))
            .collect();

        per_pattern
            .iter()
            .multi_cartesian_product()
            .filter_map(|combo| {
                let mut merged = base_bindings.clone();
                for binding_set in combo {
                    for (k, v) in binding_set {
                        if let Some(existing) = merged.get(k) {
                            if existing != v {
                                return None;
                            }
                        } else {
                            merged.insert(k.clone(), *v);
                        }
                    }
                }
                Some(merged)
            })
            .collect()
    }

    fn match_single_path(&self, pattern: &PathPattern, base_bindings: &Bindings) -> Vec<Bindings> {
        match self.strategy {
            MatchStrategy::Backtrack => self.match_path_backtrack(pattern, base_bindings),
            MatchStrategy::Fast => self.match_path_fast(pattern, base_bindings),
        }
    }

    fn match_path_backtrack(
        &self,
        pattern: &PathPattern,
        base_bindings: &Bindings,
    ) -> Vec<Bindings> {
        let mut results = Vec::new();
        let start_candidates = self.find_matching_nodes(&pattern.start, base_bindings);

        for &start_node in &start_candidates {
            let mut bindings = base_bindings.clone();
            if let Some(var) = &pattern.start.variable {
                bindings.insert(var.clone(), BoundValue::Node(start_node));
            }
            self.match_path_hops(pattern, 0, start_node, &mut bindings, &mut results);
        }

        results
    }

    fn match_path_hops(
        &self,
        pattern: &PathPattern,
        hop_index: usize,
        current_node: NodeIndex,
        bindings: &mut Bindings,
        results: &mut Vec<Bindings>,
    ) {
        if hop_index >= pattern.rels.len() {
            results.push(bindings.clone());
            return;
        }

        let (rel, target_node) = &pattern.rels[hop_index];

        let candidate_edges: Vec<_> = match rel.direction {
            RelDirection::Right => self
                .graph
                .edges_directed(current_node, petgraph::Direction::Outgoing)
                .filter(|e| Self::edge_matches_rel(e, rel))
                .map(|e| (e, e.target()))
                .collect(),
            RelDirection::Left => self
                .graph
                .edges_directed(current_node, petgraph::Direction::Incoming)
                .filter(|e| Self::edge_matches_rel(e, rel))
                .map(|e| (e, e.source()))
                .collect(),
            RelDirection::Both => {
                let out = self
                    .graph
                    .edges_directed(current_node, petgraph::Direction::Outgoing)
                    .filter(|e| Self::edge_matches_rel(e, rel))
                    .map(|e| (e, e.target()));
                let incoming = self
                    .graph
                    .edges_directed(current_node, petgraph::Direction::Incoming)
                    .filter(|e| Self::edge_matches_rel(e, rel))
                    .map(|e| (e, e.source()));
                out.chain(incoming).collect()
            }
        };

        for (edge_ref, actual_target) in candidate_edges {
            if self.node_matches_pattern(actual_target, target_node) {
                let mut new_bindings = bindings.clone();

                if let Some(var) = &rel.variable {
                    let edge_val = BoundValue::Edge(edge_ref.id());
                    if let Some(existing) = new_bindings.get(var) {
                        if existing != &edge_val {
                            continue;
                        }
                    } else {
                        new_bindings.insert(var.clone(), edge_val);
                    }
                }
                if let Some(var) = &target_node.variable {
                    let node_val = BoundValue::Node(actual_target);
                    if let Some(existing) = new_bindings.get(var) {
                        if existing != &node_val {
                            continue;
                        }
                    } else {
                        new_bindings.insert(var.clone(), node_val);
                    }
                }

                self.match_path_hops(
                    pattern,
                    hop_index + 1,
                    actual_target,
                    &mut new_bindings,
                    results,
                );
            }
        }
    }

    fn match_path_fast(&self, pattern: &PathPattern, base_bindings: &Bindings) -> Vec<Bindings> {
        self.match_path_backtrack(pattern, base_bindings)
    }

    fn find_matching_nodes(
        &self,
        pattern: &NodePattern,
        base_bindings: &Bindings,
    ) -> Vec<NodeIndex> {
        if let Some(var) = &pattern.variable {
            if let Some(&BoundValue::Node(idx)) = base_bindings.get(var) {
                if self.node_matches_pattern(idx, pattern) {
                    return vec![idx];
                }
                return vec![];
            }
        }

        self.graph
            .node_indices()
            .filter(|&idx| self.node_matches_pattern(idx, pattern))
            .collect()
    }

    fn node_matches_pattern(&self, node_idx: NodeIndex, pattern: &NodePattern) -> bool {
        let node_data = &self.graph[node_idx];
        if !pattern
            .labels
            .iter()
            .all(|label| node_data.has_label(label))
        {
            return false;
        }
        for (key, value) in &pattern.properties {
            if node_data.get(key) != Some(value) {
                return false;
            }
        }
        true
    }

    fn edge_matches_rel(edge_ref: &EdgeReference<'_, EdgeData>, rel: &RelPattern) -> bool {
        let edge_data = edge_ref.weight();
        if let Some(ref rel_type) = rel.rel_type {
            if !edge_data.has_rel_type(rel_type) {
                return false;
            }
        }
        for (key, value) in &rel.properties {
            if edge_data.get(key) != Some(value) {
                return false;
            }
        }
        true
    }

    fn evaluate_where(&self, expr: &WhereExpr, bindings: &Bindings) -> bool {
        match expr {
            WhereExpr::Eq(expression, value) => match expression {
                Expression::Property(var, prop) => {
                    if let Some(&bound) = bindings.get(var) {
                        match bound {
                            BoundValue::Node(idx) => {
                                let node_data = &self.graph[idx];
                                node_data.get(prop).map(|v| v == value).unwrap_or(false)
                            }
                            BoundValue::Edge(idx) => {
                                let edge_data = &self.graph[idx];
                                edge_data.get(prop).map(|v| v == value).unwrap_or(false)
                            }
                        }
                    } else {
                        false
                    }
                }
                Expression::Variable(_) => false,
                Expression::All => false,
            },
            WhereExpr::And(left, right) => {
                self.evaluate_where(left, bindings) && self.evaluate_where(right, bindings)
            }
            WhereExpr::Or(left, right) => {
                self.evaluate_where(left, bindings) || self.evaluate_where(right, bindings)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read-only executor
// ---------------------------------------------------------------------------

struct ReadQueryExecutor<'a> {
    matcher: PatternMatcher<'a>,
}

impl<'a> ReadQueryExecutor<'a> {
    fn new(graph: &'a Graph<NodeData, EdgeData>, strategy: MatchStrategy) -> Self {
        Self {
            matcher: PatternMatcher::new(graph, strategy),
        }
    }

    fn execute(self, query: CypherQuery) -> Result<QueryResult<'a>, CypherError> {
        let mut bindings: Vec<Bindings> = vec![HashMap::new()];
        let mut return_items: Option<Vec<ReturnItem>> = None;

        for clause in query.clauses {
            match clause {
                Clause::Match {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        bindings.retain(|b| self.matcher.evaluate_where(&where_expr, b));
                    }
                }
                Clause::Return { items } => {
                    return_items = Some(items);
                }
                Clause::Create { .. } | Clause::Merge { .. } | Clause::Delete { .. } => {
                    unreachable!("validated by validate_read_query")
                }
            }
        }

        let columns: Vec<String>;
        let items: Vec<ReturnItem>;

        if let Some(ri) = return_items {
            columns = ri
                .iter()
                .filter_map(|item| {
                    if let Some(ref alias) = item.alias {
                        Some(alias.clone())
                    } else {
                        match &item.expression {
                            Expression::Variable(v) => Some(v.clone()),
                            Expression::Property(v, p) => Some(format!("{v}.{p}")),
                            Expression::All => None,
                        }
                    }
                })
                .collect();
            items = ri;
        } else {
            columns = vec![];
            items = vec![];
        }

        Ok(QueryResult::new(
            columns,
            bindings,
            self.matcher.graph,
            items,
        ))
    }

    fn execute_match(
        &self,
        patterns: Vec<PathPattern>,
        input_bindings: Vec<Bindings>,
    ) -> Vec<Bindings> {
        if patterns.is_empty() {
            return input_bindings;
        }

        let mut results = Vec::new();
        for existing_bindings in &input_bindings {
            let pattern_results = self.matcher.match_patterns(&patterns, existing_bindings);
            results.extend(pattern_results);
        }
        results
    }
}

// ---------------------------------------------------------------------------
// Mutating executor
// ---------------------------------------------------------------------------

struct MutQueryExecutor<'a> {
    graph: &'a mut Graph<NodeData, EdgeData>,
}

impl<'a> MutQueryExecutor<'a> {
    fn new(graph: &'a mut Graph<NodeData, EdgeData>) -> Self {
        Self { graph }
    }

    fn execute(mut self, query: CypherQuery) -> Result<(), CypherError> {
        let mut bindings: Vec<Bindings> = vec![HashMap::new()];

        for clause in query.clauses {
            match clause {
                Clause::Match {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        let matcher = PatternMatcher::new(self.graph, MatchStrategy::Backtrack);
                        bindings.retain(|b| matcher.evaluate_where(&where_expr, b));
                    }
                }
                Clause::Create { patterns } => {
                    for bindings_row in &mut bindings {
                        for pattern in &patterns {
                            self.apply_path_pattern_mut(bindings_row, pattern)?;
                        }
                    }
                }
                Clause::Merge { pattern } => {
                    for bindings_row in &mut bindings {
                        let matcher = PatternMatcher::new(self.graph, MatchStrategy::Backtrack);
                        let test_results = matcher.match_single_path(&pattern, bindings_row);
                        if test_results.is_empty() {
                            self.apply_path_pattern_mut(bindings_row, &pattern)?;
                        } else {
                            // Propagate matched bindings into bindings_row
                            let first = &test_results[0];
                            for (k, v) in first {
                                if let Some(existing) = bindings_row.get(k) {
                                    if existing != v {
                                        return Err(CypherError::InvalidQuery(format!(
                                            "MERGE binding conflict on '{}': existing value differs from matched value",
                                            k
                                        )));
                                    }
                                } else {
                                    bindings_row.insert(k.clone(), *v);
                                }
                            }
                        }
                    }
                }
                Clause::Delete { variables, detach } => {
                    self.execute_delete(&variables, detach, &bindings)?;
                }
                Clause::Return { .. } => {
                    unreachable!("validated by validate_mutation_query")
                }
            }
        }

        Ok(())
    }

    fn execute_match(
        &self,
        patterns: Vec<PathPattern>,
        input_bindings: Vec<Bindings>,
    ) -> Vec<Bindings> {
        if patterns.is_empty() {
            return input_bindings;
        }

        let matcher = PatternMatcher::new(self.graph, MatchStrategy::Backtrack);
        let mut results = Vec::new();
        for existing_bindings in &input_bindings {
            let pattern_results = matcher.match_patterns(&patterns, existing_bindings);
            results.extend(pattern_results);
        }
        results
    }

    fn execute_delete(
        &mut self,
        variables: &[String],
        detach: bool,
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for var in variables {
            let bound_values: Vec<BoundValue> = bindings
                .iter()
                .filter_map(|b| b.get(var).copied())
                .unique()
                .collect();

            if bound_values.is_empty() {
                continue;
            }

            let (nodes, edges): (Vec<_>, Vec<_>) = bound_values
                .into_iter()
                .partition(|b| matches!(b, BoundValue::Node(_)));

            for edge_idx in edges {
                if let BoundValue::Edge(idx) = edge_idx {
                    self.graph.remove_edge(idx);
                }
            }

            for node_idx in nodes {
                if let BoundValue::Node(idx) = node_idx {
                    let out_edges: Vec<_> = self
                        .graph
                        .edges_directed(idx, petgraph::Direction::Outgoing)
                        .map(|e| e.id())
                        .collect();
                    let in_edges: Vec<_> = self
                        .graph
                        .edges_directed(idx, petgraph::Direction::Incoming)
                        .map(|e| e.id())
                        .collect();
                    let has_incident_edges = !out_edges.is_empty() || !in_edges.is_empty();

                    if !detach && has_incident_edges {
                        return Err(CypherError::InvalidQuery(format!(
                            "Cannot delete node '{}' because it has incident edges. Use DETACH DELETE.",
                            var
                        )));
                    }

                    if detach {
                        for edge_id in out_edges.into_iter().chain(in_edges) {
                            self.graph.remove_edge(edge_id);
                        }
                    }
                    self.graph.remove_node(idx);
                }
            }
        }
        Ok(())
    }

    fn apply_path_pattern_mut(
        &mut self,
        var_map: &mut Bindings,
        pattern: &PathPattern,
    ) -> Result<(), CypherError> {
        let mut prev_idx = self.get_or_add_node_mut(var_map, &pattern.start)?;

        for (rel, target_node) in &pattern.rels {
            let target_idx = self.get_or_add_node_mut(var_map, target_node)?;

            let edge_data = EdgeData::from_cypher(
                rel.variable.clone(),
                rel.rel_type.clone(),
                rel.properties.clone(),
            );

            let edge_id = match rel.direction {
                RelDirection::Right => self.graph.add_edge(prev_idx, target_idx, edge_data),
                RelDirection::Left => self.graph.add_edge(target_idx, prev_idx, edge_data),
                RelDirection::Both => self.graph.add_edge(prev_idx, target_idx, edge_data),
            };

            if let Some(var) = &rel.variable {
                let edge_val = BoundValue::Edge(edge_id);
                if let Some(existing) = var_map.get(var) {
                    if existing != &edge_val {
                        return Err(CypherError::InvalidQuery(format!(
                            "Variable '{}' is already bound to a different value",
                            var
                        )));
                    }
                } else {
                    var_map.insert(var.clone(), edge_val);
                }
            }

            prev_idx = target_idx;
        }
        Ok(())
    }

    fn get_or_add_node_mut(
        &mut self,
        var_map: &mut Bindings,
        pattern: &NodePattern,
    ) -> Result<NodeIndex, CypherError> {
        if let Some(var) = &pattern.variable {
            if let Some(&existing) = var_map.get(var) {
                match existing {
                    BoundValue::Node(idx) => return Ok(idx),
                    BoundValue::Edge(_) => {
                        return Err(CypherError::InvalidQuery(format!(
                            "Variable '{}' is bound to an edge, cannot use as node",
                            var
                        )));
                    }
                }
            }
        }

        let data = NodeData::from_cypher(
            pattern.variable.clone(),
            pattern.labels.clone(),
            pattern.properties.clone(),
        );

        let idx = self.graph.add_node(data);

        if let Some(var) = &pattern.variable {
            var_map.insert(var.clone(), BoundValue::Node(idx));
        }

        Ok(idx)
    }
}
