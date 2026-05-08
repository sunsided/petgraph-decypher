//! Query execution engine for running Cypher queries against a petgraph.

use std::cmp::Ordering;
use std::collections::HashMap;

use itertools::Itertools;
use petgraph::Graph;
use petgraph::graph::{EdgeIndex, EdgeReference, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::ast::*;
use crate::error::CypherError;
use crate::{EdgeData, NodeData};

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

/// Query result rows, pre-computed eagerly.
pub struct QueryResult {
    columns: Vec<String>,
    rows: std::vec::IntoIter<Row>,
}

impl QueryResult {
    fn new(columns: Vec<String>, rows: Vec<Row>) -> Self {
        Self {
            columns,
            rows: rows.into_iter(),
        }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    fn expression_display_name(expr: &Expression) -> String {
        match expr {
            Expression::Variable(v) => v.clone(),
            Expression::Property(v, p) => format!("{v}.{p}"),
            Expression::NestedProperty {
                variable,
                properties,
            } => {
                let mut name = variable.clone();
                for p in properties {
                    name.push('.');
                    name.push_str(p);
                }
                name
            }
            Expression::All => "*".to_string(),
            Expression::Literal(_) => "literal".to_string(),
            Expression::Add(a, b)
            | Expression::Sub(a, b)
            | Expression::Mul(a, b)
            | Expression::Div(a, b)
            | Expression::Mod(a, b) => {
                format!(
                    "{} ? {}",
                    Self::expression_display_name(a),
                    Self::expression_display_name(b)
                )
            }
            Expression::List(_) => "[...]".to_string(),
            Expression::Map(_) => "{...}".to_string(),
            Expression::FunctionCall { name, .. } => format!("{name}()"),
            Expression::Parameter(p) => format!("${p}"),
            Expression::Case { .. } => "CASE".to_string(),
            Expression::Aggregation { kind, .. } => {
                let kind_name = match kind {
                    AggregationKind::Count => "count",
                    AggregationKind::Sum => "sum",
                    AggregationKind::Avg => "avg",
                    AggregationKind::Min => "min",
                    AggregationKind::Max => "max",
                    AggregationKind::Collect => "collect",
                };
                format!("{kind_name}()")
            }
            Expression::Gt(a, b) => format!(
                "{} > {}",
                Self::expression_display_name(a),
                Self::expression_display_name(b)
            ),
            Expression::Lt(a, b) => format!(
                "{} < {}",
                Self::expression_display_name(a),
                Self::expression_display_name(b)
            ),
            Expression::Gte(a, b) => format!(
                "{} >= {}",
                Self::expression_display_name(a),
                Self::expression_display_name(b)
            ),
            Expression::Lte(a, b) => format!(
                "{} <= {}",
                Self::expression_display_name(a),
                Self::expression_display_name(b)
            ),
            Expression::Neq(a, b) => format!(
                "{} <> {}",
                Self::expression_display_name(a),
                Self::expression_display_name(b)
            ),
        }
    }
}

impl Iterator for QueryResult {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        self.rows.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
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
    fn cypher(&self, query: &str) -> Result<QueryResult, CypherError>;

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
    ) -> Result<QueryResult, CypherError>;
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
            Clause::SetProperty { .. }
            | Clause::SetMerge { .. }
            | Clause::SetAddLabels { .. } => {
                return Err(CypherError::Unsupported(
                    "SET not allowed in cypher(); use cypher_mut()".into(),
                ));
            }
            Clause::RemoveProperty { .. } | Clause::RemoveLabels { .. } => {
                return Err(CypherError::Unsupported(
                    "REMOVE not allowed in cypher(); use cypher_mut()".into(),
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

impl PetgraphCypher for Graph<NodeData, EdgeData> {
    fn cypher(&self, query: &str) -> Result<QueryResult, CypherError> {
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
    ) -> Result<QueryResult, CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_read_query(&ast)?;
        ReadQueryExecutor::new(self, strategy).execute(ast)
    }
}
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

        if let Some(vl) = &rel.variable_length {
            let min_hops = vl.min.unwrap_or(0);
            let max_hops = vl.max.unwrap_or(u64::MAX);
            self.match_variable_length(
                pattern, hop_index, current_node, bindings, results,
                rel, target_node, min_hops, max_hops, 0,
            );
        } else {
            self.match_fixed_hop(
                pattern, hop_index, current_node, bindings, results, rel, target_node,
            );
        }
    }

    fn match_fixed_hop(
        &self,
        pattern: &PathPattern,
        hop_index: usize,
        current_node: NodeIndex,
        bindings: &mut Bindings,
        results: &mut Vec<Bindings>,
        rel: &RelPattern,
        target_node: &NodePattern,
    ) {
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
                        if existing != &edge_val { continue; }
                    } else {
                        new_bindings.insert(var.clone(), edge_val);
                    }
                }
                if let Some(var) = &target_node.variable {
                    let node_val = BoundValue::Node(actual_target);
                    if let Some(existing) = new_bindings.get(var) {
                        if existing != &node_val { continue; }
                    } else {
                        new_bindings.insert(var.clone(), node_val);
                    }
                }
                self.match_path_hops(
                    pattern, hop_index + 1, actual_target, &mut new_bindings, results,
                );
            }
        }
    }

    fn match_variable_length(
        &self,
        pattern: &PathPattern,
        hop_index: usize,
        current_node: NodeIndex,
        bindings: &Bindings,
        results: &mut Vec<Bindings>,
        rel: &RelPattern,
        target_node: &NodePattern,
        min_hops: u64,
        max_hops: u64,
        depth: u64,
    ) {
        // Guard: prevent infinite traversal
        if depth > 1000 {
            return;
        }

        // Check if current node matches target and we're within the range
        if depth >= min_hops && self.node_matches_pattern(current_node, target_node) {
            let mut new_bindings = bindings.clone();
            if let Some(var) = &target_node.variable {
                let node_val = BoundValue::Node(current_node);
                if let Some(existing) = new_bindings.get(var) {
                    if existing != &node_val {
                        // Binding conflict — skip this path
                    } else {
                        new_bindings.insert(var.clone(), node_val);
                        self.match_path_hops(
                            pattern, hop_index + 1, current_node, &mut new_bindings, results,
                        );
                    }
                } else {
                    new_bindings.insert(var.clone(), node_val);
                    self.match_path_hops(
                        pattern, hop_index + 1, current_node, &mut new_bindings, results,
                    );
                }
            } else {
                self.match_path_hops(
                    pattern, hop_index + 1, current_node, &mut new_bindings, results,
                );
            }
        }

        // Continue traversing if we haven't hit max
        if depth < max_hops {
            let mut visited = std::collections::HashSet::new();
            visited.insert(current_node);
            self.traverse_variable_length(
                pattern, hop_index, current_node, bindings, results,
                rel, target_node, min_hops, max_hops, depth, &mut visited,
            );
        }
    }

    fn traverse_variable_length(
        &self,
        pattern: &PathPattern,
        hop_index: usize,
        current_node: NodeIndex,
        bindings: &Bindings,
        results: &mut Vec<Bindings>,
        rel: &RelPattern,
        target_node: &NodePattern,
        min_hops: u64,
        max_hops: u64,
        depth: u64,
        visited: &mut std::collections::HashSet<NodeIndex>,
    ) {
        let edges: Vec<_> = match rel.direction {
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

        for (_edge_ref, next_node) in edges {
            if visited.contains(&next_node) {
                continue;
            }
            let new_depth = depth + 1;
            visited.insert(next_node);

            // If this node matches target and we're in range, produce a result
            if new_depth >= min_hops && self.node_matches_pattern(next_node, target_node) {
                let mut new_bindings = bindings.clone();
                if let Some(var) = &target_node.variable {
                    let node_val = BoundValue::Node(next_node);
                    if let Some(existing) = new_bindings.get(var) {
                        if existing != &node_val {
                            visited.remove(&next_node);
                            continue;
                        }
                    }
                    new_bindings.insert(var.clone(), node_val);
                }
                self.match_path_hops(
                    pattern, hop_index + 1, next_node, &mut new_bindings, results,
                );
            }

            if new_depth < max_hops {
                self.traverse_variable_length(
                    pattern, hop_index, next_node, bindings, results,
                    rel, target_node, min_hops, max_hops, new_depth, visited,
                );
            }
            visited.remove(&next_node);
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
        if !pattern.labels.iter().all(|label| {
            node_data
                .labels
                .iter()
                .any(|node_label| node_label == label)
        }) {
            return false;
        }
        for (key, value) in &pattern.properties {
            if node_data.properties.get(key) != Some(value) {
                return false;
            }
        }
        true
    }

    fn edge_matches_rel(edge_ref: &EdgeReference<'_, EdgeData>, rel: &RelPattern) -> bool {
        let edge_data = edge_ref.weight();
        if !rel.rel_types.is_empty() {
            if let Some(ref edge_type) = edge_data.rel_type {
                if !rel.rel_types.iter().any(|t| t == edge_type) {
                    return false;
                }
            } else {
                return false;
            }
        }
        for (key, value) in &rel.properties {
            if edge_data.properties.get(key) != Some(value) {
                return false;
            }
        }
        true
    }

    fn evaluate_where(&self, expr: &WhereExpr, bindings: &Bindings) -> bool {
        match expr {
            WhereExpr::Eq(expression, value) => {
                let actual = eval_expression_to_value(self.graph, expression, bindings);
                actual.as_ref() == Some(value)
            }
            WhereExpr::Neq(expression, value) => {
                let actual = eval_expression_to_value(self.graph, expression, bindings);
                actual.as_ref() != Some(value)
            }
            WhereExpr::Lt(expression, value) => {
                let actual = eval_expression_to_number(self.graph, expression, bindings);
                let expected = cypher_value_to_number(value);
                match (actual, expected) {
                    (Some(a), Some(b)) => a < b,
                    _ => false,
                }
            }
            WhereExpr::Gt(expression, value) => {
                let actual = eval_expression_to_number(self.graph, expression, bindings);
                let expected = cypher_value_to_number(value);
                match (actual, expected) {
                    (Some(a), Some(b)) => a > b,
                    _ => false,
                }
            }
            WhereExpr::Lte(expression, value) => {
                let actual = eval_expression_to_number(self.graph, expression, bindings);
                let expected = cypher_value_to_number(value);
                match (actual, expected) {
                    (Some(a), Some(b)) => a <= b,
                    _ => false,
                }
            }
            WhereExpr::Gte(expression, value) => {
                let actual = eval_expression_to_number(self.graph, expression, bindings);
                let expected = cypher_value_to_number(value);
                match (actual, expected) {
                    (Some(a), Some(b)) => a >= b,
                    _ => false,
                }
            }
            WhereExpr::In(_expression, _list_expr) => {
                // List membership not yet fully supported as a WHERE filter
                false
            }
            WhereExpr::IsNull(expression) => {
                matches!(
                    eval_expression_to_value(self.graph, expression, bindings),
                    None | Some(CypherValue::Null)
                )
            }
            WhereExpr::IsNotNull(expression) => !matches!(
                eval_expression_to_value(self.graph, expression, bindings),
                None | Some(CypherValue::Null)
            ),
            WhereExpr::StartsWith(left, right) => {
                let a = eval_expression_to_string(self.graph, left, bindings);
                let b = eval_expression_to_string(self.graph, right, bindings);
                match (a, b) {
                    (Some(a_str), Some(b_str)) => a_str.starts_with(&b_str),
                    _ => false,
                }
            }
            WhereExpr::EndsWith(left, right) => {
                let a = eval_expression_to_string(self.graph, left, bindings);
                let b = eval_expression_to_string(self.graph, right, bindings);
                match (a, b) {
                    (Some(a_str), Some(b_str)) => a_str.ends_with(&b_str),
                    _ => false,
                }
            }
            WhereExpr::Contains(left, right) => {
                let a = eval_expression_to_string(self.graph, left, bindings);
                let b = eval_expression_to_string(self.graph, right, bindings);
                match (a, b) {
                    (Some(a_str), Some(b_str)) => a_str.contains(&b_str),
                    _ => false,
                }
            }
            WhereExpr::Not(inner) => !self.evaluate_where(inner, bindings),
            WhereExpr::And(left, right) => {
                self.evaluate_where(left, bindings) && self.evaluate_where(right, bindings)
            }
            WhereExpr::Or(left, right) => {
                self.evaluate_where(left, bindings) || self.evaluate_where(right, bindings)
            }
            WhereExpr::Expr(expr) => {
                matches!(
                    eval_expression_to_value(self.graph, expr, bindings),
                    Some(CypherValue::Boolean(true))
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Eager result helpers (Phase 3)
// ---------------------------------------------------------------------------

fn project_row(
    graph: &Graph<NodeData, EdgeData>,
    bindings: &Bindings,
    items: &[ReturnItem],
) -> Row {
    let mut values = HashMap::new();

    for item in items {
        let key = if let Some(ref alias) = item.alias {
            alias.clone()
        } else {
            QueryResult::expression_display_name(&item.expression)
        };

        if matches!(item.expression, Expression::All) {
            for (var, bound) in bindings {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[*idx];
                        values.insert(
                            var.clone(),
                            ResultValue::Node {
                                labels: data.labels.clone(),
                                properties: data.properties.clone(),
                            },
                        );
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[*idx];
                        values.insert(
                            var.clone(),
                            ResultValue::Edge {
                                rel_type: data.rel_type.clone(),
                                properties: data.properties.clone(),
                            },
                        );
                    }
                }
            }
            continue;
        }

        let result_value = evaluate_expression_for_return(graph, &item.expression, bindings);
        values.insert(key, result_value);
    }

    Row { values }
}

fn evaluate_expression_for_return(
    graph: &Graph<NodeData, EdgeData>,
    expr: &Expression,
    bindings: &Bindings,
) -> ResultValue {
    match expr {
        Expression::Variable(var) => {
            if let Some(&bound) = bindings.get(var) {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[idx];
                        ResultValue::Node {
                            labels: data.labels.clone(),
                            properties: data.properties.clone(),
                        }
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[idx];
                        ResultValue::Edge {
                            rel_type: data.rel_type.clone(),
                            properties: data.properties.clone(),
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
                    BoundValue::Node(idx) => graph[idx]
                        .properties
                        .get(prop)
                        .cloned()
                        .map(ResultValue::Scalar)
                        .unwrap_or(ResultValue::Scalar(CypherValue::Null)),
                    BoundValue::Edge(idx) => graph[idx]
                        .properties
                        .get(prop)
                        .cloned()
                        .map(ResultValue::Scalar)
                        .unwrap_or(ResultValue::Scalar(CypherValue::Null)),
                }
            } else {
                ResultValue::Scalar(CypherValue::Null)
            }
        }
        Expression::NestedProperty { variable, properties } => {
            if let Some(&bound) = bindings.get(variable) {
                let props = match bound {
                    BoundValue::Node(idx) => &graph[idx].properties,
                    BoundValue::Edge(idx) => &graph[idx].properties,
                };
                if properties.is_empty() {
                    ResultValue::Scalar(CypherValue::Null)
                } else {
                    props
                        .get(&properties[0])
                        .cloned()
                        .map(ResultValue::Scalar)
                        .unwrap_or(ResultValue::Scalar(CypherValue::Null))
                }
            } else {
                ResultValue::Scalar(CypherValue::Null)
            }
        }
        Expression::All => unreachable!(),
        Expression::Literal(val) => ResultValue::Scalar(val.clone()),
        Expression::Add(l, r) => eval_arithmetic_return(graph, l, r, |a, b| a + b, bindings),
        Expression::Sub(l, r) => eval_arithmetic_return(graph, l, r, |a, b| a - b, bindings),
        Expression::Mul(l, r) => eval_arithmetic_return(graph, l, r, |a, b| a * b, bindings),
        Expression::Div(l, r) => eval_arithmetic_return(graph, l, r, |a, b| {
            if b == 0.0 { f64::NAN } else { a / b }
        }, bindings),
        Expression::Mod(l, r) => eval_arithmetic_return(graph, l, r, |a, b| a % b, bindings),
        Expression::List(_) => ResultValue::Scalar(CypherValue::Null),
        Expression::Map(_) => ResultValue::Scalar(CypherValue::Null),
        Expression::FunctionCall { .. } => ResultValue::Scalar(CypherValue::Null),
        Expression::Parameter(_) => ResultValue::Scalar(CypherValue::Null),
        Expression::Case { .. } => ResultValue::Scalar(CypherValue::Null),
        Expression::Aggregation { .. } => ResultValue::Scalar(CypherValue::Null),
        Expression::Gt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings);
            let b = eval_expression_to_number(graph, r, bindings);
            match (a, b) {
                (Some(a), Some(b)) => ResultValue::Scalar(CypherValue::Boolean(a > b)),
                _ => ResultValue::Scalar(CypherValue::Null),
            }
        }
        Expression::Lt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings);
            let b = eval_expression_to_number(graph, r, bindings);
            match (a, b) {
                (Some(a), Some(b)) => ResultValue::Scalar(CypherValue::Boolean(a < b)),
                _ => ResultValue::Scalar(CypherValue::Null),
            }
        }
        Expression::Gte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings);
            let b = eval_expression_to_number(graph, r, bindings);
            match (a, b) {
                (Some(a), Some(b)) => ResultValue::Scalar(CypherValue::Boolean(a >= b)),
                _ => ResultValue::Scalar(CypherValue::Null),
            }
        }
        Expression::Lte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings);
            let b = eval_expression_to_number(graph, r, bindings);
            match (a, b) {
                (Some(a), Some(b)) => ResultValue::Scalar(CypherValue::Boolean(a <= b)),
                _ => ResultValue::Scalar(CypherValue::Null),
            }
        }
        Expression::Neq(l, r) => {
            let a = eval_expression_to_value(graph, l, bindings);
            let b = eval_expression_to_value(graph, r, bindings);
            ResultValue::Scalar(CypherValue::Boolean(a != b))
        }
    }
}

fn eval_arithmetic_return<F>(
    graph: &Graph<NodeData, EdgeData>,
    left: &Expression,
    right: &Expression,
    op: F,
    bindings: &Bindings,
) -> ResultValue
where
    F: Fn(f64, f64) -> f64,
{
    let l = eval_expression_to_number(graph, left, bindings);
    let r = eval_expression_to_number(graph, right, bindings);
    match (l, r) {
        (Some(a), Some(b)) => {
            let result = op(a, b);
            if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                ResultValue::Scalar(CypherValue::Integer(result as i64))
            } else {
                ResultValue::Scalar(CypherValue::Float(result))
            }
        }
        _ => ResultValue::Scalar(CypherValue::Null),
    }
}

fn compute_columns(items: &[ReturnItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| {
            if let Some(ref alias) = item.alias {
                Some(alias.clone())
            } else {
                match &item.expression {
                    Expression::Variable(v) => Some(v.clone()),
                    Expression::Property(v, p) => Some(format!("{v}.{p}")),
                    Expression::NestedProperty { variable, properties } => {
                        let mut name = variable.clone();
                        for p in properties {
                            name.push('.');
                            name.push_str(p);
                        }
                        Some(name)
                    }
                    Expression::All => None,
                    Expression::Literal(_) => Some("literal".to_string()),
                    Expression::Add(..)
                    | Expression::Sub(..)
                    | Expression::Mul(..)
                    | Expression::Div(..)
                    | Expression::Mod(..) => Some("expr".to_string()),
                    Expression::List(_) => Some("list".to_string()),
                    Expression::Map(_) => Some("map".to_string()),
                    Expression::FunctionCall { name, .. } => Some(name.clone()),
                    Expression::Parameter(p) => Some(p.clone()),
                    Expression::Case { .. } => Some("case".to_string()),
                    Expression::Aggregation { kind, .. } => {
                        let kind_name = match kind {
                            AggregationKind::Count => "count",
                            AggregationKind::Sum => "sum",
                            AggregationKind::Avg => "avg",
                            AggregationKind::Min => "min",
                            AggregationKind::Max => "max",
                            AggregationKind::Collect => "collect",
                        };
                        Some(kind_name.to_string())
                    }
                    Expression::Gt(..)
                    | Expression::Lt(..)
                    | Expression::Gte(..)
                    | Expression::Lte(..)
                    | Expression::Neq(..) => Some("comparison".to_string()),
                }
            }
        })
        .collect()
}

fn deduplicate_rows(paired: Vec<(Bindings, Row)>) -> Vec<(Bindings, Row)> {
    let mut result: Vec<(Bindings, Row)> = Vec::with_capacity(paired.len());
    for (bindings, row) in paired {
        let is_duplicate = result.iter().any(|(_, existing)| existing.values == row.values);
        if !is_duplicate {
            result.push((bindings, row));
        }
    }
    result
}

fn compare_order_by(
    order_by: &[OrderByItem],
    b1: &Bindings,
    b2: &Bindings,
    graph: &Graph<NodeData, EdgeData>,
) -> Ordering {
    for item in order_by {
        let v1 = eval_expression_to_value(graph, &item.expression, b1);
        let v2 = eval_expression_to_value(graph, &item.expression, b2);
        let cmp = compare_cypher_values(&v1, &v2);
        if cmp != Ordering::Equal {
            return match item.direction {
                OrderDirection::Ascending => cmp,
                OrderDirection::Descending => cmp.reverse(),
            };
        }
    }
    Ordering::Equal
}

fn compare_cypher_values(a: &Option<CypherValue>, b: &Option<CypherValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => compare_cypher_value_inner(a, b),
    }
}

fn compare_cypher_value_inner(a: &CypherValue, b: &CypherValue) -> Ordering {
    match (a, b) {
        (CypherValue::Integer(a), CypherValue::Integer(b)) => a.cmp(b),
        (CypherValue::Float(a), CypherValue::Float(b)) => {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (CypherValue::Integer(a), CypherValue::Float(b)) => {
            (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (CypherValue::Float(a), CypherValue::Integer(b)) => {
            a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
        }
        (CypherValue::String(a), CypherValue::String(b)) => a.cmp(b),
        (CypherValue::Boolean(a), CypherValue::Boolean(b)) => a.cmp(b),
        (CypherValue::Null, CypherValue::Null) => Ordering::Equal,
        (CypherValue::Null, _) => Ordering::Less,
        (_, CypherValue::Null) => Ordering::Greater,
        _ => {
            let type_order = |v: &CypherValue| -> u8 {
                match v {
                    CypherValue::Null => 0,
                    CypherValue::Boolean(_) => 1,
                    CypherValue::Integer(_) => 2,
                    CypherValue::Float(_) => 3,
                    CypherValue::String(_) => 4,
                }
            };
            type_order(a).cmp(&type_order(b))
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

    fn execute(self, query: CypherQuery) -> Result<QueryResult, CypherError> {
        let mut bindings: Vec<Bindings> = vec![HashMap::new()];
        let mut return_items: Option<(Vec<ReturnItem>, bool)> = None;

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
                Clause::Return { items, distinct } => {
                    return_items = Some((items, distinct));
                }
                Clause::Create { .. }
                | Clause::Merge { .. }
                | Clause::Delete { .. }
                | Clause::SetProperty { .. }
                | Clause::SetMerge { .. }
                | Clause::SetAddLabels { .. }
                | Clause::RemoveProperty { .. }
                | Clause::RemoveLabels { .. } => {
                    unreachable!("validated by validate_read_query")
                }
            }
        }

        let (items, distinct) = return_items.unwrap_or_else(|| (vec![], false));
        let columns = compute_columns(&items);
        let graph = self.matcher.graph;

        let mut paired: Vec<(Bindings, Row)> = bindings
            .into_iter()
            .map(|b| {
                let row = project_row(graph, &b, &items);
                (b, row)
            })
            .collect();

        if distinct {
            paired = deduplicate_rows(paired);
        }

        if !query.order_by.is_empty() {
            let ob = query.order_by;
            paired.sort_by(|(b1, _), (b2, _)| {
                compare_order_by(&ob, b1, b2, graph)
            });
        }

        let start = query.skip.unwrap_or(0) as usize;
        let remaining = if start < paired.len() {
            &paired[start..]
        } else {
            &[]
        };
        let take_count = match query.limit {
            Some(l) => remaining.len().min(l as usize),
            None => remaining.len(),
        };

        let rows: Vec<Row> = remaining[..take_count]
            .iter()
            .map(|(_, row)| row.clone())
            .collect();

        Ok(QueryResult::new(columns, rows))
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
                Clause::SetProperty {
                    variable,
                    property,
                    value,
                } => {
                    self.execute_set_property(&variable, &property, &value, &bindings)?;
                }
                Clause::SetMerge {
                    variable,
                    properties,
                } => {
                    self.execute_set_merge(&variable, &properties, &bindings)?;
                }
                Clause::SetAddLabels { variable, labels } => {
                    self.execute_set_labels(&variable, &labels, &bindings)?;
                }
                Clause::RemoveProperty { variable, property } => {
                    self.execute_remove_property(&variable, &property, &bindings)?;
                }
                Clause::RemoveLabels { variable, labels } => {
                    self.execute_remove_labels(&variable, &labels, &bindings)?;
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

            let edge_data = EdgeData {
                variable: rel.variable.clone(),
                rel_type: rel.rel_types.first().cloned(),
                properties: rel.properties.clone(),
            };

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

        let data = NodeData {
            variable: pattern.variable.clone(),
            labels: pattern.labels.clone(),
            properties: pattern.properties.clone(),
        };

        let idx = self.graph.add_node(data);

        if let Some(var) = &pattern.variable {
            var_map.insert(var.clone(), BoundValue::Node(idx));
        }

        Ok(idx)
    }

    fn execute_set_property(
        &mut self,
        variable: &str,
        property: &str,
        value: &CypherValue,
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for bindings_row in bindings {
            if let Some(&bound) = bindings_row.get(variable) {
                match bound {
                    BoundValue::Node(idx) => {
                        let node_data = &mut self.graph[idx];
                        node_data
                            .properties
                            .insert(property.to_string(), value.clone());
                    }
                    BoundValue::Edge(idx) => {
                        let edge_data = &mut self.graph[idx];
                        edge_data
                            .properties
                            .insert(property.to_string(), value.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_set_merge(
        &mut self,
        variable: &str,
        properties: &HashMap<String, CypherValue>,
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for bindings_row in bindings {
            if let Some(&bound) = bindings_row.get(variable) {
                match bound {
                    BoundValue::Node(idx) => {
                        let node_data = &mut self.graph[idx];
                        for (k, v) in properties {
                            node_data.properties.insert(k.clone(), v.clone());
                        }
                    }
                    BoundValue::Edge(idx) => {
                        let edge_data = &mut self.graph[idx];
                        for (k, v) in properties {
                            edge_data.properties.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_set_labels(
        &mut self,
        variable: &str,
        labels: &[String],
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for bindings_row in bindings {
            if let Some(&bound) = bindings_row.get(variable) {
                if let BoundValue::Node(idx) = bound {
                    let node_data = &mut self.graph[idx];
                    for label in labels {
                        if !node_data.labels.contains(label) {
                            node_data.labels.push(label.clone());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_remove_property(
        &mut self,
        variable: &str,
        property: &str,
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for bindings_row in bindings {
            if let Some(&bound) = bindings_row.get(variable) {
                match bound {
                    BoundValue::Node(idx) => {
                        let node_data = &mut self.graph[idx];
                        node_data.properties.remove(property);
                    }
                    BoundValue::Edge(idx) => {
                        let edge_data = &mut self.graph[idx];
                        edge_data.properties.remove(property);
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_remove_labels(
        &mut self,
        variable: &str,
        labels: &[String],
        bindings: &[Bindings],
    ) -> Result<(), CypherError> {
        for bindings_row in bindings {
            if let Some(&bound) = bindings_row.get(variable) {
                if let BoundValue::Node(idx) = bound {
                    let node_data = &mut self.graph[idx];
                    node_data.labels.retain(|l| !labels.contains(l));
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Standalone expression evaluation helpers
// ---------------------------------------------------------------------------

fn cypher_value_to_number(val: &CypherValue) -> Option<f64> {
    match val {
        CypherValue::Integer(i) => Some(*i as f64),
        CypherValue::Float(f) => Some(*f),
        _ => None,
    }
}

fn resolve_bound_to_number(graph: &Graph<NodeData, EdgeData>, bound: &BoundValue) -> Option<f64> {
    match bound {
        BoundValue::Node(idx) => graph[*idx]
            .properties
            .values()
            .next()
            .and_then(cypher_value_to_number),
        BoundValue::Edge(idx) => graph[*idx]
            .properties
            .values()
            .next()
            .and_then(cypher_value_to_number),
    }
}

/// Extract a `CypherValue` from an `Expression` given the current bindings.
fn eval_expression_to_value<'a>(
    graph: &'a Graph<NodeData, EdgeData>,
    expr: &Expression,
    bindings: &Bindings,
) -> Option<CypherValue> {
    match expr {
        Expression::Literal(v) => Some(v.clone()),
        Expression::Property(var, prop) => bindings.get(var).and_then(|bound| match bound {
            BoundValue::Node(idx) => graph[*idx].properties.get(prop).cloned(),
            BoundValue::Edge(idx) => graph[*idx].properties.get(prop).cloned(),
        }),
        Expression::NestedProperty {
            variable,
            properties,
        } => bindings.get(variable).and_then(|bound| {
            let props = match bound {
                BoundValue::Node(idx) => &graph[*idx].properties,
                BoundValue::Edge(idx) => &graph[*idx].properties,
            };
            if properties.is_empty() {
                None
            } else {
                props.get(&properties[0]).cloned()
            }
        }),
        Expression::Variable(_) => None,
        Expression::All => None,
        Expression::Add(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            let result = a + b;
            if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                Some(CypherValue::Integer(result as i64))
            } else {
                Some(CypherValue::Float(result))
            }
        }
        Expression::Sub(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            let result = a - b;
            if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                Some(CypherValue::Integer(result as i64))
            } else {
                Some(CypherValue::Float(result))
            }
        }
        Expression::Mul(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            let result = a * b;
            if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                Some(CypherValue::Integer(result as i64))
            } else {
                Some(CypherValue::Float(result))
            }
        }
        Expression::Div(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if b == 0.0 {
                None
            } else {
                let result = a / b;
                if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                    Some(CypherValue::Integer(result as i64))
                } else {
                    Some(CypherValue::Float(result))
                }
            }
        }
        Expression::Mod(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            let result = a % b;
            if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
                Some(CypherValue::Integer(result as i64))
            } else {
                Some(CypherValue::Float(result))
            }
        }
        Expression::List(_) => None,
        Expression::Map(_) => None,
        Expression::FunctionCall { .. } => None,
        Expression::Parameter(_) => None,
        Expression::Case { .. } => None,
        Expression::Aggregation { .. } => None,
        Expression::Gt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(CypherValue::Boolean(a > b))
        }
        Expression::Lt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(CypherValue::Boolean(a < b))
        }
        Expression::Gte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(CypherValue::Boolean(a >= b))
        }
        Expression::Lte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(CypherValue::Boolean(a <= b))
        }
        Expression::Neq(l, r) => {
            let a = eval_expression_to_value(graph, l, bindings);
            let b = eval_expression_to_value(graph, r, bindings);
            Some(CypherValue::Boolean(a != b))
        }
    }
}

fn eval_expression_to_number(
    graph: &Graph<NodeData, EdgeData>,
    expr: &Expression,
    bindings: &Bindings,
) -> Option<f64> {
    match expr {
        Expression::Literal(CypherValue::Integer(i)) => Some(*i as f64),
        Expression::Literal(CypherValue::Float(f)) => Some(*f),
        Expression::Literal(_) => None,
        Expression::Add(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(a + b)
        }
        Expression::Sub(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(a - b)
        }
        Expression::Mul(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(a * b)
        }
        Expression::Div(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if b == 0.0 { None } else { Some(a / b) }
        }
        Expression::Mod(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            Some(a % b)
        }
        Expression::Property(var, prop) => bindings.get(var).and_then(|bound| match bound {
            BoundValue::Node(idx) => graph[*idx]
                .properties
                .get(prop)
                .and_then(cypher_value_to_number),
            BoundValue::Edge(idx) => graph[*idx]
                .properties
                .get(prop)
                .and_then(cypher_value_to_number),
        }),
        Expression::NestedProperty {
            variable,
            properties,
        } => bindings.get(variable).and_then(|bound| {
            let props = match bound {
                BoundValue::Node(idx) => &graph[*idx].properties,
                BoundValue::Edge(idx) => &graph[*idx].properties,
            };
            if properties.is_empty() {
                None
            } else {
                props.get(&properties[0]).and_then(cypher_value_to_number)
            }
        }),
        Expression::Variable(var) => bindings
            .get(var)
            .and_then(|bound| resolve_bound_to_number(graph, bound)),
        Expression::All => None,
        Expression::List(_) => None,
        Expression::Map(_) => None,
        Expression::FunctionCall { .. } => None,
        Expression::Parameter(_) => None,
        Expression::Case { .. } => None,
        Expression::Aggregation { .. } => None,
        Expression::Gt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if a > b { Some(1.0) } else { Some(0.0) }
        }
        Expression::Lt(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if a < b { Some(1.0) } else { Some(0.0) }
        }
        Expression::Gte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if a >= b { Some(1.0) } else { Some(0.0) }
        }
        Expression::Lte(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if a <= b { Some(1.0) } else { Some(0.0) }
        }
        Expression::Neq(l, r) => {
            let a = eval_expression_to_number(graph, l, bindings)?;
            let b = eval_expression_to_number(graph, r, bindings)?;
            if a != b { Some(1.0) } else { Some(0.0) }
        }
    }
}

fn eval_expression_to_string(
    graph: &Graph<NodeData, EdgeData>,
    expr: &Expression,
    bindings: &Bindings,
) -> Option<String> {
    match expr {
        Expression::Literal(CypherValue::String(s)) => Some(s.clone()),
        Expression::Property(var, prop) => bindings.get(var).and_then(|bound| match bound {
            BoundValue::Node(idx) => graph[*idx].properties.get(prop).and_then(|v| match v {
                CypherValue::String(s) => Some(s.clone()),
                _ => None,
            }),
            BoundValue::Edge(idx) => graph[*idx].properties.get(prop).and_then(|v| match v {
                CypherValue::String(s) => Some(s.clone()),
                _ => None,
            }),
        }),
        Expression::Variable(var) => bindings.get(var).and_then(|bound| match bound {
            BoundValue::Node(idx) => graph[*idx].properties.get("name").and_then(|v| match v {
                CypherValue::String(s) => Some(s.clone()),
                _ => None,
            }),
            BoundValue::Edge(idx) => graph[*idx].properties.get("name").and_then(|v| match v {
                CypherValue::String(s) => Some(s.clone()),
                _ => None,
            }),
        }),
        _ => None,
    }
}
