//! Query execution engine for running Cypher queries against a petgraph.

use std::collections::HashMap;

use itertools::Itertools;
use petgraph::Graph;
use petgraph::graph::{EdgeIndex, EdgeReference, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::ast::*;
use crate::error::CypherError;
use crate::{CypherEdge, CypherNode, EdgeData, NodeData};

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
    /// A scalar property value (including List and Map).
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
/// Query parameters mapped by name (without the leading `$`).
///
/// For example, query text `$name` is resolved from the `"name"` key.
pub type Parameters = HashMap<String, CypherValue>;

/// An iterator over query result rows.
///
/// Both the MATCH and RETURN projection phases are executed eagerly.
/// The returned iterator streams over the fully materialized result rows.
pub struct QueryResult<'a, N = NodeData, E = EdgeData> {
    columns: Vec<String>,
    rows: std::vec::IntoIter<Row>,
    _graph: &'a Graph<N, E>,
}

impl<'a, N, E> QueryResult<'a, N, E> {
    fn new(columns: Vec<String>, rows: Vec<Row>, graph: &'a Graph<N, E>) -> Self {
        Self {
            columns,
            rows: rows.into_iter(),
            _graph: graph,
        }
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }
}

impl<'a, N, E> Iterator for QueryResult<'a, N, E> {
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
    fn cypher(&self, query: &str) -> Result<QueryResult<'_>, CypherError>;

    /// Execute a read-only Cypher query and stream the results using parameters.
    fn cypher_params(
        &self,
        query: &str,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_>, CypherError> {
        if parameters.is_empty() {
            self.cypher(query)
        } else {
            Err(CypherError::Unsupported(
                "parameterized queries are not supported by this PetgraphCypher implementor".into(),
            ))
        }
    }

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

    /// Execute a read-only Cypher query with a specific match strategy and parameters.
    fn cypher_with_strategy_params(
        &self,
        query: &str,
        strategy: MatchStrategy,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_>, CypherError> {
        if parameters.is_empty() {
            self.cypher_with_strategy(query, strategy)
        } else {
            Err(CypherError::Unsupported(
                "parameterized queries are not supported by this PetgraphCypher implementor".into(),
            ))
        }
    }
}

/// Extension trait for executing read-only Cypher queries on a
/// [`Graph`] with custom node and edge weights.
pub trait PetgraphCypherRead<N: CypherNode, E: CypherEdge> {
    /// Execute a read-only Cypher query and stream the results.
    ///
    /// Supports `MATCH`, `WHERE`, and `RETURN` clauses.
    /// Returns an error if mutation clauses (`CREATE`, `MERGE`, `DELETE`) are present.
    ///
    /// Uses the default `Backtrack` match strategy.
    fn cypher(&self, query: &str) -> Result<QueryResult<'_, N, E>, CypherError>;

    /// Execute a read-only Cypher query and stream the results using parameters.
    fn cypher_params(
        &self,
        query: &str,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_, N, E>, CypherError> {
        if parameters.is_empty() {
            self.cypher(query)
        } else {
            Err(CypherError::Unsupported(
                "parameterized queries are not supported by this PetgraphCypherRead implementor"
                    .into(),
            ))
        }
    }

    /// Execute a read-only Cypher query with a specific match strategy.
    fn cypher_with_strategy(
        &self,
        query: &str,
        strategy: MatchStrategy,
    ) -> Result<QueryResult<'_, N, E>, CypherError>;

    /// Execute a read-only Cypher query with a specific match strategy and parameters.
    fn cypher_with_strategy_params(
        &self,
        query: &str,
        strategy: MatchStrategy,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_, N, E>, CypherError> {
        if parameters.is_empty() {
            self.cypher_with_strategy(query, strategy)
        } else {
            Err(CypherError::Unsupported(
                "parameterized queries are not supported by this PetgraphCypherRead implementor"
                    .into(),
            ))
        }
    }
}

impl<N: CypherNode, E: CypherEdge> PetgraphCypherRead<N, E> for Graph<N, E> {
    fn cypher(&self, query: &str) -> Result<QueryResult<'_, N, E>, CypherError> {
        let params = Parameters::new();
        self.cypher_with_strategy_params(query, MatchStrategy::default(), &params)
    }

    fn cypher_params(
        &self,
        query: &str,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_, N, E>, CypherError> {
        self.cypher_with_strategy_params(query, MatchStrategy::default(), parameters)
    }

    fn cypher_with_strategy(
        &self,
        query: &str,
        strategy: MatchStrategy,
    ) -> Result<QueryResult<'_, N, E>, CypherError> {
        let params = Parameters::new();
        self.cypher_with_strategy_params(query, strategy, &params)
    }

    fn cypher_with_strategy_params(
        &self,
        query: &str,
        strategy: MatchStrategy,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_, N, E>, CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_read_query(&ast)?;
        validate_read_query_parameters(&ast, parameters)?;
        ReadQueryExecutor::new(self, strategy, parameters).execute(ast)
    }
}

impl PetgraphCypher for Graph<NodeData, EdgeData> {
    fn cypher(&self, query: &str) -> Result<QueryResult<'_>, CypherError> {
        let params = Parameters::new();
        PetgraphCypher::cypher_with_strategy_params(self, query, MatchStrategy::default(), &params)
    }

    fn cypher_params(
        &self,
        query: &str,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_>, CypherError> {
        PetgraphCypher::cypher_with_strategy_params(
            self,
            query,
            MatchStrategy::default(),
            parameters,
        )
    }

    fn cypher_mut(&mut self, query: &str) -> Result<(), CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_mutation_query(&ast)?;
        validate_mutation_query_parameters(&ast)?;
        MutQueryExecutor::new(self).execute(ast)
    }

    fn cypher_with_strategy(
        &self,
        query: &str,
        strategy: MatchStrategy,
    ) -> Result<QueryResult<'_>, CypherError> {
        let params = Parameters::new();
        PetgraphCypher::cypher_with_strategy_params(self, query, strategy, &params)
    }

    fn cypher_with_strategy_params(
        &self,
        query: &str,
        strategy: MatchStrategy,
        parameters: &Parameters,
    ) -> Result<QueryResult<'_>, CypherError> {
        let ast = crate::parse_cypher(query)?;
        validate_read_query(&ast)?;
        validate_read_query_parameters(&ast, parameters)?;
        ReadQueryExecutor::new(self, strategy, parameters).execute(ast)
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
            Clause::Set { .. } | Clause::Remove { .. } => {
                return Err(CypherError::Unsupported(
                    "SET/REMOVE not allowed in cypher(); use cypher_mut()".into(),
                ));
            }
            Clause::With { .. } => {
                return Err(CypherError::Unsupported("WITH is not yet supported".into()));
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

fn ensure_expression_parameters(
    expression: &Expression,
    parameters: &Parameters,
) -> Result<(), CypherError> {
    match expression {
        Expression::Unary(_, inner) => ensure_expression_parameters(inner, parameters),
        Expression::Binary(_, left, right) => {
            ensure_expression_parameters(left, parameters)?;
            ensure_expression_parameters(right, parameters)
        }
        Expression::List(items) => {
            for item in items {
                ensure_expression_parameters(item, parameters)?;
            }
            Ok(())
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                ensure_expression_parameters(arg, parameters)?;
            }
            Ok(())
        }
        Expression::Parameter(name) => {
            if parameters.contains_key(name) {
                Ok(())
            } else {
                Err(CypherError::InvalidQuery(format!(
                    "missing query parameter: ${name}"
                )))
            }
        }
        Expression::Case(case) => {
            if let Some(scrutinee) = &case.scrutinee {
                ensure_expression_parameters(scrutinee, parameters)?;
            }
            for (when, then) in &case.alternatives {
                ensure_expression_parameters(when, parameters)?;
                ensure_expression_parameters(then, parameters)?;
            }
            if let Some(default) = &case.default {
                ensure_expression_parameters(default, parameters)?;
            }
            Ok(())
        }
        Expression::Variable(_)
        | Expression::Property(_, _)
        | Expression::All
        | Expression::Literal(_) => Ok(()),
    }
}

fn ensure_where_parameters(
    where_expr: &WhereExpr,
    parameters: &Parameters,
) -> Result<(), CypherError> {
    match where_expr {
        WhereExpr::Eq(left, right)
        | WhereExpr::NotEq(left, right)
        | WhereExpr::Lt(left, right)
        | WhereExpr::Gt(left, right)
        | WhereExpr::Le(left, right)
        | WhereExpr::Ge(left, right) => {
            ensure_expression_parameters(left, parameters)?;
            ensure_expression_parameters(right, parameters)
        }
        WhereExpr::In(expression, _)
        | WhereExpr::StartsWith(expression, _)
        | WhereExpr::EndsWith(expression, _)
        | WhereExpr::Contains(expression, _)
        | WhereExpr::IsNull(expression)
        | WhereExpr::IsNotNull(expression) => ensure_expression_parameters(expression, parameters),
        WhereExpr::Not(inner) => ensure_where_parameters(inner, parameters),
        WhereExpr::And(left, right) | WhereExpr::Or(left, right) | WhereExpr::Xor(left, right) => {
            ensure_where_parameters(left, parameters)?;
            ensure_where_parameters(right, parameters)
        }
    }
}

fn validate_read_query_parameters(
    query: &CypherQuery,
    parameters: &Parameters,
) -> Result<(), CypherError> {
    for clause in &query.clauses {
        match clause {
            Clause::Match { where_clause, .. } | Clause::OptionalMatch { where_clause, .. } => {
                if let Some(where_expr) = where_clause {
                    ensure_where_parameters(where_expr, parameters)?;
                }
            }
            Clause::Return { items, .. } => {
                for item in items {
                    ensure_expression_parameters(&item.expression, parameters)?;
                }
            }
            Clause::Unwind { expression, .. } => {
                ensure_expression_parameters(expression, parameters)?;
            }
            Clause::OrderBy { items } => {
                for item in items {
                    ensure_expression_parameters(&item.expression, parameters)?;
                }
            }
            Clause::Create { .. }
            | Clause::With { .. }
            | Clause::Set { .. }
            | Clause::Merge { .. }
            | Clause::Delete { .. }
            | Clause::Remove { .. }
            | Clause::Skip { .. }
            | Clause::Limit { .. } => {}
        }
    }
    Ok(())
}

fn ensure_expression_has_no_parameters(expression: &Expression) -> Result<(), CypherError> {
    match expression {
        Expression::Unary(_, inner) => ensure_expression_has_no_parameters(inner),
        Expression::Binary(_, left, right) => {
            ensure_expression_has_no_parameters(left)?;
            ensure_expression_has_no_parameters(right)
        }
        Expression::List(items) => {
            for item in items {
                ensure_expression_has_no_parameters(item)?;
            }
            Ok(())
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                ensure_expression_has_no_parameters(arg)?;
            }
            Ok(())
        }
        Expression::Parameter(name) => Err(CypherError::Unsupported(format!(
            "query parameters are not supported in cypher_mut(): ${name}"
        ))),
        Expression::Case(case) => {
            if let Some(scrutinee) = &case.scrutinee {
                ensure_expression_has_no_parameters(scrutinee)?;
            }
            for (when, then) in &case.alternatives {
                ensure_expression_has_no_parameters(when)?;
                ensure_expression_has_no_parameters(then)?;
            }
            if let Some(default) = &case.default {
                ensure_expression_has_no_parameters(default)?;
            }
            Ok(())
        }
        Expression::Variable(_)
        | Expression::Property(_, _)
        | Expression::All
        | Expression::Literal(_) => Ok(()),
    }
}

fn ensure_where_has_no_parameters(where_expr: &WhereExpr) -> Result<(), CypherError> {
    match where_expr {
        WhereExpr::Eq(left, right)
        | WhereExpr::NotEq(left, right)
        | WhereExpr::Lt(left, right)
        | WhereExpr::Gt(left, right)
        | WhereExpr::Le(left, right)
        | WhereExpr::Ge(left, right) => {
            ensure_expression_has_no_parameters(left)?;
            ensure_expression_has_no_parameters(right)
        }
        WhereExpr::In(expression, _)
        | WhereExpr::StartsWith(expression, _)
        | WhereExpr::EndsWith(expression, _)
        | WhereExpr::Contains(expression, _)
        | WhereExpr::IsNull(expression)
        | WhereExpr::IsNotNull(expression) => ensure_expression_has_no_parameters(expression),
        WhereExpr::Not(inner) => ensure_where_has_no_parameters(inner),
        WhereExpr::And(left, right) | WhereExpr::Or(left, right) | WhereExpr::Xor(left, right) => {
            ensure_where_has_no_parameters(left)?;
            ensure_where_has_no_parameters(right)
        }
    }
}

fn ensure_set_item_has_no_parameters(item: &SetItem) -> Result<(), CypherError> {
    match item {
        SetItem::SetProperty { value, .. } => ensure_expression_has_no_parameters(value),
        SetItem::SetVariable { .. }
        | SetItem::SetLabels { .. }
        | SetItem::MergeProperties { .. } => Ok(()),
    }
}

fn validate_mutation_query_parameters(query: &CypherQuery) -> Result<(), CypherError> {
    for clause in &query.clauses {
        match clause {
            Clause::Match { where_clause, .. } | Clause::OptionalMatch { where_clause, .. } => {
                if let Some(where_expr) = where_clause {
                    ensure_where_has_no_parameters(where_expr)?;
                }
            }
            Clause::Set { items } => {
                for item in items {
                    ensure_set_item_has_no_parameters(item)?;
                }
            }
            Clause::Merge {
                on_create,
                on_match,
                ..
            } => {
                for item in on_create {
                    ensure_set_item_has_no_parameters(item)?;
                }
                for item in on_match {
                    ensure_set_item_has_no_parameters(item)?;
                }
            }
            Clause::Unwind { expression, .. } => ensure_expression_has_no_parameters(expression)?,
            Clause::Create { .. }
            | Clause::Delete { .. }
            | Clause::Remove { .. }
            | Clause::Return { .. }
            | Clause::With { .. }
            | Clause::OrderBy { .. }
            | Clause::Skip { .. }
            | Clause::Limit { .. } => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Expression evaluation helpers
// ---------------------------------------------------------------------------

fn is_truthy(value: &CypherValue) -> Option<bool> {
    match value {
        CypherValue::Null => None,
        CypherValue::Boolean(b) => Some(*b),
        _ => Some(true),
    }
}

fn compare_cypher_values(left: &CypherValue, right: &CypherValue) -> Option<std::cmp::Ordering> {
    match (left, right) {
        (CypherValue::Null, _) | (_, CypherValue::Null) => None,
        (CypherValue::Boolean(a), CypherValue::Boolean(b)) => a.partial_cmp(b),
        (CypherValue::Integer(a), CypherValue::Integer(b)) => a.partial_cmp(b),
        (CypherValue::Integer(a), CypherValue::Float(b)) => (*a as f64).partial_cmp(b),
        (CypherValue::Float(a), CypherValue::Integer(b)) => a.partial_cmp(&(*b as f64)),
        (CypherValue::Float(a), CypherValue::Float(b)) => a.partial_cmp(b),
        (CypherValue::String(a), CypherValue::String(b)) => a.partial_cmp(b),
        (CypherValue::List(a), CypherValue::List(b)) => a.partial_cmp(b),
        _ => None,
    }
}

fn cypher_value_eq(left: &CypherValue, right: &CypherValue) -> CypherValue {
    match (left, right) {
        (CypherValue::Null, _) | (_, CypherValue::Null) => CypherValue::Null,
        _ => CypherValue::Boolean(left == right),
    }
}

fn evaluate_binary_op(
    op: BinaryOp,
    left: CypherValue,
    right: CypherValue,
) -> Result<CypherValue, CypherError> {
    match op {
        BinaryOp::Add => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => Ok(CypherValue::Integer(a + b)),
            (CypherValue::Float(a), CypherValue::Float(b)) => Ok(CypherValue::Float(a + b)),
            (CypherValue::Integer(a), CypherValue::Float(b)) => {
                Ok(CypherValue::Float(*a as f64 + b))
            }
            (CypherValue::Float(a), CypherValue::Integer(b)) => {
                Ok(CypherValue::Float(a + *b as f64))
            }
            (CypherValue::String(a), CypherValue::String(b)) => {
                Ok(CypherValue::String(format!("{}{}", a, b)))
            }
            (CypherValue::List(a), CypherValue::List(b)) => {
                let mut result = a.clone();
                result.extend(b.clone());
                Ok(CypherValue::List(result))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot add {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Subtract => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => Ok(CypherValue::Integer(a - b)),
            (CypherValue::Float(a), CypherValue::Float(b)) => Ok(CypherValue::Float(a - b)),
            (CypherValue::Integer(a), CypherValue::Float(b)) => {
                Ok(CypherValue::Float(*a as f64 - b))
            }
            (CypherValue::Float(a), CypherValue::Integer(b)) => {
                Ok(CypherValue::Float(a - *b as f64))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot subtract {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Multiply => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => Ok(CypherValue::Integer(a * b)),
            (CypherValue::Float(a), CypherValue::Float(b)) => Ok(CypherValue::Float(a * b)),
            (CypherValue::Integer(a), CypherValue::Float(b)) => {
                Ok(CypherValue::Float(*a as f64 * b))
            }
            (CypherValue::Float(a), CypherValue::Integer(b)) => {
                Ok(CypherValue::Float(a * *b as f64))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot multiply {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Divide => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => {
                if *b == 0 {
                    return Err(CypherError::DivisionByZero);
                }
                Ok(CypherValue::Float(*a as f64 / *b as f64))
            }
            (CypherValue::Float(a), CypherValue::Float(b)) => {
                if *b == 0.0 {
                    return Err(CypherError::DivisionByZero);
                }
                Ok(CypherValue::Float(a / b))
            }
            (CypherValue::Integer(a), CypherValue::Float(b)) => {
                if *b == 0.0 {
                    return Err(CypherError::DivisionByZero);
                }
                Ok(CypherValue::Float(*a as f64 / b))
            }
            (CypherValue::Float(a), CypherValue::Integer(b)) => {
                if *b == 0 {
                    return Err(CypherError::DivisionByZero);
                }
                Ok(CypherValue::Float(a / *b as f64))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot divide {:?} by {:?}",
                left, right
            ))),
        },
        BinaryOp::Modulo => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => {
                if *b == 0 {
                    return Err(CypherError::DivisionByZero);
                }
                Ok(CypherValue::Integer(a % b))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot modulo {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Power => match (&left, &right) {
            (CypherValue::Integer(a), CypherValue::Integer(b)) => {
                if *b < 0 {
                    return Err(CypherError::TypeMismatch(
                        "negative integer exponents are not supported for integer base; use a float base".into(),
                    ));
                }
                if *b > u32::MAX as i64 {
                    return Err(CypherError::TypeMismatch(
                        "integer exponent is too large".into(),
                    ));
                }
                Ok(CypherValue::Integer(a.pow(*b as u32)))
            }
            (CypherValue::Float(a), CypherValue::Float(b)) => Ok(CypherValue::Float(a.powf(*b))),
            (CypherValue::Integer(a), CypherValue::Float(b)) => {
                Ok(CypherValue::Float((*a as f64).powf(*b)))
            }
            (CypherValue::Float(a), CypherValue::Integer(b)) => {
                Ok(CypherValue::Float(a.powf(*b as f64)))
            }
            _ => Err(CypherError::TypeMismatch(format!(
                "cannot power {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Eq => Ok(cypher_value_eq(&left, &right)),
        BinaryOp::Ne => {
            let eq = cypher_value_eq(&left, &right);
            match eq {
                CypherValue::Null => Ok(CypherValue::Null),
                CypherValue::Boolean(b) => Ok(CypherValue::Boolean(!b)),
                _ => unreachable!(),
            }
        }
        BinaryOp::Lt => Ok(match compare_cypher_values(&left, &right) {
            None => CypherValue::Null,
            Some(ord) => CypherValue::Boolean(ord == std::cmp::Ordering::Less),
        }),
        BinaryOp::Gt => Ok(match compare_cypher_values(&left, &right) {
            None => CypherValue::Null,
            Some(ord) => CypherValue::Boolean(ord == std::cmp::Ordering::Greater),
        }),
        BinaryOp::Le => Ok(match compare_cypher_values(&left, &right) {
            None => CypherValue::Null,
            Some(ord) => CypherValue::Boolean(ord != std::cmp::Ordering::Greater),
        }),
        BinaryOp::Ge => Ok(match compare_cypher_values(&left, &right) {
            None => CypherValue::Null,
            Some(ord) => CypherValue::Boolean(ord != std::cmp::Ordering::Less),
        }),
        BinaryOp::And => match (is_truthy(&left), is_truthy(&right)) {
            (Some(false), _) | (_, Some(false)) => Ok(CypherValue::Boolean(false)),
            (Some(true), Some(true)) => Ok(CypherValue::Boolean(true)),
            _ => Ok(CypherValue::Null),
        },
        BinaryOp::Or => match (is_truthy(&left), is_truthy(&right)) {
            (Some(true), _) | (_, Some(true)) => Ok(CypherValue::Boolean(true)),
            (Some(false), Some(false)) => Ok(CypherValue::Boolean(false)),
            _ => Ok(CypherValue::Null),
        },
        BinaryOp::Xor => match (is_truthy(&left), is_truthy(&right)) {
            (Some(a), Some(b)) => Ok(CypherValue::Boolean(a ^ b)),
            _ => Ok(CypherValue::Null),
        },
        BinaryOp::StartsWith => match (&left, &right) {
            (CypherValue::String(a), CypherValue::String(b)) => {
                Ok(CypherValue::Boolean(a.starts_with(b)))
            }
            (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
            _ => Err(CypherError::TypeMismatch(format!(
                "STARTS WITH requires strings, got {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::EndsWith => match (&left, &right) {
            (CypherValue::String(a), CypherValue::String(b)) => {
                Ok(CypherValue::Boolean(a.ends_with(b)))
            }
            (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
            _ => Err(CypherError::TypeMismatch(format!(
                "ENDS WITH requires strings, got {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::Contains => match (&left, &right) {
            (CypherValue::String(a), CypherValue::String(b)) => {
                Ok(CypherValue::Boolean(a.contains(b)))
            }
            (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
            _ => Err(CypherError::TypeMismatch(format!(
                "CONTAINS requires strings, got {:?} and {:?}",
                left, right
            ))),
        },
        BinaryOp::In => match (&right, &left) {
            (CypherValue::List(list), _) => {
                let found = list.iter().any(|item| item == &left);
                Ok(CypherValue::Boolean(found))
            }
            (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
            _ => Err(CypherError::TypeMismatch(format!(
                "IN requires a list on the right side, got {:?}",
                right
            ))),
        },
    }
}

fn evaluate_function(
    name: &str,
    args: &[Expression],
    _distinct: bool,
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> Result<CypherValue, CypherError> {
    // Helper to evaluate all args.
    let eval_args = || {
        args.iter()
            .map(|arg| evaluate_expression(arg, bindings, graph, parameters))
            .collect::<Result<Vec<_>, _>>()
    };

    match name.to_lowercase().as_str() {
        "tostring" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "toString() takes exactly 1 argument".into(),
                ));
            }
            let s = match &vals[0] {
                CypherValue::String(s) => s.clone(),
                CypherValue::Integer(i) => i.to_string(),
                CypherValue::Float(f) => f.to_string(),
                CypherValue::Boolean(b) => b.to_string(),
                CypherValue::Null => "null".to_string(),
                CypherValue::List(_) | CypherValue::Map(_) => {
                    return Err(CypherError::TypeMismatch(
                        "toString() does not support lists or maps".into(),
                    ));
                }
            };
            Ok(CypherValue::String(s))
        }
        "tointeger" | "toint" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "toInteger() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Integer(i) => Ok(CypherValue::Integer(*i)),
                CypherValue::Float(f) => Ok(CypherValue::Integer(*f as i64)),
                CypherValue::String(s) => {
                    s.parse::<i64>().map(CypherValue::Integer).map_err(|_| {
                        CypherError::TypeMismatch(format!("cannot convert '{}' to integer", s))
                    })
                }
                CypherValue::Boolean(true) => Ok(CypherValue::Integer(1)),
                CypherValue::Boolean(false) => Ok(CypherValue::Integer(0)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "cannot convert {:?} to integer",
                    vals[0]
                ))),
            }
        }
        "tofloat" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "toFloat() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Float(f) => Ok(CypherValue::Float(*f)),
                CypherValue::Integer(i) => Ok(CypherValue::Float(*i as f64)),
                CypherValue::String(s) => s.parse::<f64>().map(CypherValue::Float).map_err(|_| {
                    CypherError::TypeMismatch(format!("cannot convert '{}' to float", s))
                }),
                CypherValue::Boolean(true) => Ok(CypherValue::Float(1.0)),
                CypherValue::Boolean(false) => Ok(CypherValue::Float(0.0)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "cannot convert {:?} to float",
                    vals[0]
                ))),
            }
        }
        "coalesce" => {
            for arg in args {
                let val = evaluate_expression(arg, bindings, graph, parameters)?;
                if !matches!(val, CypherValue::Null) {
                    return Ok(val);
                }
            }
            Ok(CypherValue::Null)
        }
        "head" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "head() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::List(list) => Ok(list.first().cloned().unwrap_or(CypherValue::Null)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "head() requires a list, got {:?}",
                    vals[0]
                ))),
            }
        }
        "last" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "last() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::List(list) => Ok(list.last().cloned().unwrap_or(CypherValue::Null)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "last() requires a list, got {:?}",
                    vals[0]
                ))),
            }
        }
        "size" | "length" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(format!(
                    "{}() takes exactly 1 argument",
                    name
                )));
            }
            match &vals[0] {
                CypherValue::List(list) => Ok(CypherValue::Integer(list.len() as i64)),
                CypherValue::String(s) => Ok(CypherValue::Integer(s.len() as i64)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "{}() requires a list or string, got {:?}",
                    name, vals[0]
                ))),
            }
        }
        "toupper" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "toUpper() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.to_uppercase())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "toUpper() requires a string, got {:?}",
                    vals[0]
                ))),
            }
        }
        "tolower" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "toLower() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.to_lowercase())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "toLower() requires a string, got {:?}",
                    vals[0]
                ))),
            }
        }
        "trim" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "trim() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.trim().to_string())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "trim() requires a string, got {:?}",
                    vals[0]
                ))),
            }
        }
        "ltrim" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "ltrim() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.trim_start().to_string())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "ltrim() requires a string, got {:?}",
                    vals[0]
                ))),
            }
        }
        "rtrim" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "rtrim() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.trim_end().to_string())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "rtrim() requires a string, got {:?}",
                    vals[0]
                ))),
            }
        }
        "substring" => {
            let vals = eval_args()?;
            if vals.len() < 2 || vals.len() > 3 {
                return Err(CypherError::InvalidQuery(
                    "substring() takes 2 or 3 arguments".into(),
                ));
            }
            match (&vals[0], &vals[1]) {
                (CypherValue::String(s), CypherValue::Integer(start)) => {
                    let start = *start as usize;
                    let len = vals.get(2).and_then(|v| match v {
                        CypherValue::Integer(i) => Some(*i as usize),
                        _ => None,
                    });
                    let result = if let Some(len) = len {
                        s.chars().skip(start).take(len).collect()
                    } else {
                        s.chars().skip(start).collect()
                    };
                    Ok(CypherValue::String(result))
                }
                (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "substring() requires (string, integer[, integer]), got {:?}",
                    vals
                ))),
            }
        }
        "replace" => {
            let vals = eval_args()?;
            if vals.len() != 3 {
                return Err(CypherError::InvalidQuery(
                    "replace() takes exactly 3 arguments".into(),
                ));
            }
            match (&vals[0], &vals[1], &vals[2]) {
                (CypherValue::String(s), CypherValue::String(old), CypherValue::String(new)) => {
                    Ok(CypherValue::String(s.replace(old, new)))
                }
                (CypherValue::Null, _, _)
                | (_, CypherValue::Null, _)
                | (_, _, CypherValue::Null) => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "replace() requires (string, string, string), got {:?}",
                    vals
                ))),
            }
        }
        "split" => {
            let vals = eval_args()?;
            if vals.len() != 2 {
                return Err(CypherError::InvalidQuery(
                    "split() takes exactly 2 arguments".into(),
                ));
            }
            match (&vals[0], &vals[1]) {
                (CypherValue::String(s), CypherValue::String(delim)) => {
                    let parts: Vec<CypherValue> = s
                        .split(delim)
                        .map(|p| CypherValue::String(p.to_string()))
                        .collect();
                    Ok(CypherValue::List(parts))
                }
                (CypherValue::Null, _) | (_, CypherValue::Null) => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "split() requires (string, string), got {:?}",
                    vals
                ))),
            }
        }
        "reverse" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "reverse() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::String(s) => Ok(CypherValue::String(s.chars().rev().collect())),
                CypherValue::List(list) => {
                    let mut reversed = list.clone();
                    reversed.reverse();
                    Ok(CypherValue::List(reversed))
                }
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "reverse() requires a string or list, got {:?}",
                    vals[0]
                ))),
            }
        }
        "abs" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "abs() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Integer(i) => Ok(CypherValue::Integer(i.abs())),
                CypherValue::Float(f) => Ok(CypherValue::Float(f.abs())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "abs() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "ceil" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "ceil() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Float(f) => Ok(CypherValue::Float(f.ceil())),
                CypherValue::Integer(i) => Ok(CypherValue::Float(*i as f64)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "ceil() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "floor" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "floor() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Float(f) => Ok(CypherValue::Float(f.floor())),
                CypherValue::Integer(i) => Ok(CypherValue::Float(*i as f64)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "floor() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "round" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "round() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Float(f) => Ok(CypherValue::Float(f.round())),
                CypherValue::Integer(i) => Ok(CypherValue::Float(*i as f64)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "round() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "sign" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "sign() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Integer(i) => Ok(CypherValue::Integer(i.signum())),
                CypherValue::Float(f) => Ok(CypherValue::Integer(f.signum() as i64)),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "sign() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "sqrt" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "sqrt() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::Float(f) => Ok(CypherValue::Float(f.sqrt())),
                CypherValue::Integer(i) => Ok(CypherValue::Float((*i as f64).sqrt())),
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "sqrt() requires a number, got {:?}",
                    vals[0]
                ))),
            }
        }
        "rand" => {
            if !args.is_empty() {
                return Err(CypherError::InvalidQuery(
                    "rand() takes no arguments".into(),
                ));
            }
            #[cfg(feature = "rng-rand")]
            {
                Ok(CypherValue::Float(rand::random::<f64>()))
            }
            #[cfg(all(feature = "rng-fastrand", not(feature = "rng-rand")))]
            {
                Ok(CypherValue::Float(fastrand::f64()))
            }
            #[cfg(not(any(feature = "rng-rand", feature = "rng-fastrand")))]
            {
                Err(CypherError::Unsupported(
                    "rand() requires an RNG feature to be enabled (rng-rand or rng-fastrand)"
                        .into(),
                ))
            }
        }
        "range" => {
            let vals = eval_args()?;
            if vals.len() < 2 || vals.len() > 3 {
                return Err(CypherError::InvalidQuery(
                    "range() takes 2 or 3 arguments".into(),
                ));
            }
            let start = match &vals[0] {
                CypherValue::Integer(i) => *i,
                _ => {
                    return Err(CypherError::TypeMismatch(format!(
                        "range() requires integers, got {:?}",
                        vals
                    )));
                }
            };
            let end = match &vals[1] {
                CypherValue::Integer(i) => *i,
                _ => {
                    return Err(CypherError::TypeMismatch(format!(
                        "range() requires integers, got {:?}",
                        vals
                    )));
                }
            };
            let step = match vals.get(2) {
                Some(v) => match v {
                    CypherValue::Integer(i) => *i,
                    _ => {
                        return Err(CypherError::TypeMismatch(
                            "range() step must be an integer".into(),
                        ));
                    }
                },
                None => 1,
            };
            if step == 0 {
                return Err(CypherError::InvalidQuery(
                    "range() step cannot be zero".into(),
                ));
            }
            let mut result = Vec::new();
            let mut current = start;
            if step > 0 {
                while current <= end {
                    result.push(CypherValue::Integer(current));
                    current += step;
                }
            } else {
                while current >= end {
                    result.push(CypherValue::Integer(current));
                    current += step;
                }
            }
            Ok(CypherValue::List(result))
        }
        "tail" => {
            let vals = eval_args()?;
            if vals.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "tail() takes exactly 1 argument".into(),
                ));
            }
            match &vals[0] {
                CypherValue::List(list) => {
                    Ok(CypherValue::List(list.iter().skip(1).cloned().collect()))
                }
                CypherValue::Null => Ok(CypherValue::Null),
                _ => Err(CypherError::TypeMismatch(format!(
                    "tail() requires a list, got {:?}",
                    vals[0]
                ))),
            }
        }
        "type" => {
            if args.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "type() takes exactly 1 argument".into(),
                ));
            }
            match &args[0] {
                Expression::Variable(var) => {
                    if let Some(BoundValue::Edge(idx)) = bindings.get(var) {
                        let data = &graph[*idx];
                        Ok(data
                            .rel_type()
                            .map(|s| CypherValue::String(s.to_string()))
                            .unwrap_or(CypherValue::Null))
                    } else {
                        Ok(CypherValue::Null)
                    }
                }
                _ => Err(CypherError::TypeMismatch(
                    "type() requires a relationship variable".into(),
                )),
            }
        }
        "labels" => {
            if args.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "labels() takes exactly 1 argument".into(),
                ));
            }
            match &args[0] {
                Expression::Variable(var) => {
                    if let Some(BoundValue::Node(idx)) = bindings.get(var) {
                        let data = &graph[*idx];
                        let labels: Vec<CypherValue> =
                            data.labels().into_iter().map(CypherValue::String).collect();
                        Ok(CypherValue::List(labels))
                    } else {
                        Ok(CypherValue::Null)
                    }
                }
                _ => Err(CypherError::TypeMismatch(
                    "labels() requires a node variable".into(),
                )),
            }
        }
        "properties" => {
            if args.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "properties() takes exactly 1 argument".into(),
                ));
            }
            match &args[0] {
                Expression::Variable(var) => {
                    if let Some(bound) = bindings.get(var) {
                        let props = match bound {
                            BoundValue::Node(idx) => graph[*idx].properties(),
                            BoundValue::Edge(idx) => graph[*idx].properties(),
                        };
                        Ok(CypherValue::Map(props))
                    } else {
                        Ok(CypherValue::Null)
                    }
                }
                _ => Err(CypherError::TypeMismatch(
                    "properties() requires a variable".into(),
                )),
            }
        }
        "keys" => {
            if args.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "keys() takes exactly 1 argument".into(),
                ));
            }
            match &args[0] {
                Expression::Variable(var) => {
                    if let Some(bound) = bindings.get(var) {
                        let props = match bound {
                            BoundValue::Node(idx) => graph[*idx].properties(),
                            BoundValue::Edge(idx) => graph[*idx].properties(),
                        };
                        let keys: Vec<CypherValue> =
                            props.keys().cloned().map(CypherValue::String).collect();
                        Ok(CypherValue::List(keys))
                    } else {
                        Ok(CypherValue::Null)
                    }
                }
                _ => Err(CypherError::TypeMismatch(
                    "keys() requires a variable".into(),
                )),
            }
        }
        "exists" => {
            if args.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "exists() takes exactly 1 argument".into(),
                ));
            }
            match &args[0] {
                Expression::Property(var, prop) => {
                    let exists = if let Some(bound) = bindings.get(var) {
                        match bound {
                            BoundValue::Node(idx) => graph[*idx].get(prop).is_some(),
                            BoundValue::Edge(idx) => graph[*idx].get(prop).is_some(),
                        }
                    } else {
                        false
                    };
                    Ok(CypherValue::Boolean(exists))
                }
                _ => {
                    // For general expressions, check if not null
                    let val = evaluate_expression(&args[0], bindings, graph, parameters)?;
                    Ok(CypherValue::Boolean(!matches!(val, CypherValue::Null)))
                }
            }
        }
        _ => Err(CypherError::Unsupported(format!(
            "unsupported function: {}",
            name
        ))),
    }
}

fn evaluate_case(
    case: &CaseExpr,
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> Result<CypherValue, CypherError> {
    if let Some(scrutinee) = &case.scrutinee {
        let scrutinee_val = evaluate_expression(scrutinee, bindings, graph, parameters)?;
        for (when, then) in &case.alternatives {
            let when_val = evaluate_expression(when, bindings, graph, parameters)?;
            if scrutinee_val == when_val {
                return evaluate_expression(then, bindings, graph, parameters);
            }
        }
    } else {
        for (when, then) in &case.alternatives {
            let when_val = evaluate_expression(when, bindings, graph, parameters)?;
            if is_truthy(&when_val) == Some(true) {
                return evaluate_expression(then, bindings, graph, parameters);
            }
        }
    }
    if let Some(default) = &case.default {
        evaluate_expression(default, bindings, graph, parameters)
    } else {
        Ok(CypherValue::Null)
    }
}

fn evaluate_expression(
    expr: &Expression,
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> Result<CypherValue, CypherError> {
    match expr {
        Expression::Variable(var) => {
            if let Some(bound) = bindings.get(var) {
                match bound {
                    BoundValue::Node(_) | BoundValue::Edge(_) => {
                        Err(CypherError::TypeMismatch(format!(
                            "node/edge variable '{}' cannot be used in a scalar expression",
                            var
                        )))
                    }
                }
            } else {
                Ok(CypherValue::Null)
            }
        }
        Expression::Property(var, prop) => {
            if let Some(bound) = bindings.get(var) {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[*idx];
                        Ok(data.get(prop).cloned().unwrap_or(CypherValue::Null))
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[*idx];
                        Ok(data.get(prop).cloned().unwrap_or(CypherValue::Null))
                    }
                }
            } else {
                Ok(CypherValue::Null)
            }
        }
        Expression::Literal(v) => Ok(v.clone()),
        Expression::All => Ok(CypherValue::Null),
        Expression::Unary(op, expr) => {
            let val = evaluate_expression(expr, bindings, graph, parameters)?;
            match op {
                UnaryOp::Not => match val {
                    CypherValue::Null => Ok(CypherValue::Null),
                    _ => Ok(CypherValue::Boolean(is_truthy(&val) == Some(false))),
                },
                UnaryOp::Negate => match val {
                    CypherValue::Integer(i) => Ok(CypherValue::Integer(-i)),
                    CypherValue::Float(f) => Ok(CypherValue::Float(-f)),
                    CypherValue::Null => Ok(CypherValue::Null),
                    _ => Err(CypherError::TypeMismatch(format!(
                        "cannot negate {:?}",
                        val
                    ))),
                },
                UnaryOp::Plus => match val {
                    CypherValue::Integer(_) | CypherValue::Float(_) | CypherValue::Null => Ok(val),
                    _ => Err(CypherError::TypeMismatch(format!(
                        "cannot apply + to {:?}",
                        val
                    ))),
                },
            }
        }
        Expression::Binary(op, left, right) => {
            let left_val = evaluate_expression(left, bindings, graph, parameters)?;
            let right_val = evaluate_expression(right, bindings, graph, parameters)?;
            evaluate_binary_op(*op, left_val, right_val)
        }
        Expression::List(items) => {
            let values = items
                .iter()
                .map(|item| evaluate_expression(item, bindings, graph, parameters))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(CypherValue::List(values))
        }
        Expression::FunctionCall {
            name,
            args,
            distinct,
        } => evaluate_function(name, args, *distinct, bindings, graph, parameters),
        Expression::Parameter(name) => parameters
            .get(name)
            .cloned()
            .ok_or_else(|| CypherError::InvalidQuery(format!("missing query parameter: ${name}"))),
        Expression::Case(case) => evaluate_case(case, bindings, graph, parameters),
    }
}

fn evaluate_where(
    expr: &WhereExpr,
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> bool {
    match expr {
        WhereExpr::Eq(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => matches!(cypher_value_eq(&l, &r), CypherValue::Boolean(true)),
                _ => false,
            }
        }
        WhereExpr::NotEq(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => matches!(cypher_value_eq(&l, &r), CypherValue::Boolean(false)),
                _ => false,
            }
        }
        WhereExpr::Lt(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => compare_cypher_values(&l, &r) == Some(std::cmp::Ordering::Less),
                _ => false,
            }
        }
        WhereExpr::Gt(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => {
                    compare_cypher_values(&l, &r) == Some(std::cmp::Ordering::Greater)
                }
                _ => false,
            }
        }
        WhereExpr::Le(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => {
                    matches!(
                        compare_cypher_values(&l, &r),
                        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                    )
                }
                _ => false,
            }
        }
        WhereExpr::Ge(left, right) => {
            match (
                evaluate_expression(left, bindings, graph, parameters),
                evaluate_expression(right, bindings, graph, parameters),
            ) {
                (Ok(l), Ok(r)) => {
                    matches!(
                        compare_cypher_values(&l, &r),
                        Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                    )
                }
                _ => false,
            }
        }
        WhereExpr::In(expression, values) => {
            match evaluate_expression(expression, bindings, graph, parameters) {
                Ok(val) => values.iter().any(|v| v == &val),
                _ => false,
            }
        }
        WhereExpr::StartsWith(expression, prefix) => {
            match evaluate_expression(expression, bindings, graph, parameters) {
                Ok(CypherValue::String(s)) => s.starts_with(prefix),
                _ => false,
            }
        }
        WhereExpr::EndsWith(expression, suffix) => {
            match evaluate_expression(expression, bindings, graph, parameters) {
                Ok(CypherValue::String(s)) => s.ends_with(suffix),
                _ => false,
            }
        }
        WhereExpr::Contains(expression, substring) => {
            match evaluate_expression(expression, bindings, graph, parameters) {
                Ok(CypherValue::String(s)) => s.contains(substring),
                _ => false,
            }
        }
        WhereExpr::IsNull(expression) => {
            matches!(
                evaluate_expression(expression, bindings, graph, parameters),
                Ok(CypherValue::Null)
            )
        }
        WhereExpr::IsNotNull(expression) => !matches!(
            evaluate_expression(expression, bindings, graph, parameters),
            Ok(CypherValue::Null)
        ),
        WhereExpr::Not(inner) => !evaluate_where(inner, bindings, graph, parameters),
        WhereExpr::And(left, right) => {
            evaluate_where(left, bindings, graph, parameters)
                && evaluate_where(right, bindings, graph, parameters)
        }
        WhereExpr::Or(left, right) => {
            evaluate_where(left, bindings, graph, parameters)
                || evaluate_where(right, bindings, graph, parameters)
        }
        WhereExpr::Xor(left, right) => {
            evaluate_where(left, bindings, graph, parameters)
                ^ evaluate_where(right, bindings, graph, parameters)
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern matching
// ---------------------------------------------------------------------------

/// Shared matcher for pattern matching against a graph. Used by both the read
/// and mutation executors to avoid code duplication.
struct PatternMatcher<'a, N: CypherNode, E: CypherEdge> {
    graph: &'a Graph<N, E>,
    strategy: MatchStrategy,
}

impl<'a, N: CypherNode, E: CypherEdge> PatternMatcher<'a, N, E> {
    fn new(graph: &'a Graph<N, E>, strategy: MatchStrategy) -> Self {
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
        if let Some(var) = &pattern.variable
            && let Some(&BoundValue::Node(idx)) = base_bindings.get(var)
        {
            if self.node_matches_pattern(idx, pattern) {
                return vec![idx];
            }
            return vec![];
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

    fn edge_matches_rel(edge_ref: &EdgeReference<'_, E>, rel: &RelPattern) -> bool {
        let edge_data = edge_ref.weight();
        if let Some(ref rel_type) = rel.rel_type
            && !edge_data.has_rel_type(rel_type)
        {
            return false;
        }
        for (key, value) in &rel.properties {
            if edge_data.get(key) != Some(value) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Result projection helpers
// ---------------------------------------------------------------------------

fn project_expression(
    expr: &Expression,
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> Result<ResultValue, CypherError> {
    match expr {
        Expression::Variable(var) => {
            if let Some(&bound) = bindings.get(var) {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[idx];
                        Ok(ResultValue::Node {
                            labels: data.labels(),
                            properties: data.properties(),
                        })
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[idx];
                        Ok(ResultValue::Edge {
                            rel_type: data.rel_type().map(str::to_string),
                            properties: data.properties(),
                        })
                    }
                }
            } else {
                Ok(ResultValue::Scalar(CypherValue::Null))
            }
        }
        Expression::Property(var, prop) => {
            if let Some(&bound) = bindings.get(var) {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[idx];
                        Ok(data
                            .get(prop)
                            .cloned()
                            .map(ResultValue::Scalar)
                            .unwrap_or(ResultValue::Scalar(CypherValue::Null)))
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[idx];
                        Ok(data
                            .get(prop)
                            .cloned()
                            .map(ResultValue::Scalar)
                            .unwrap_or(ResultValue::Scalar(CypherValue::Null)))
                    }
                }
            } else {
                Ok(ResultValue::Scalar(CypherValue::Null))
            }
        }
        Expression::All => Ok(ResultValue::Scalar(CypherValue::Null)),
        other => {
            let val = evaluate_expression(other, bindings, graph, parameters)?;
            Ok(ResultValue::Scalar(val))
        }
    }
}

fn column_name(item: &ReturnItem, expr_counter: &mut usize) -> Option<String> {
    if let Some(ref alias) = item.alias {
        Some(alias.clone())
    } else {
        match &item.expression {
            Expression::Variable(v) => Some(v.clone()),
            Expression::Property(v, p) => Some(format!("{}.{}", v, p)),
            Expression::All => None,
            _ => {
                let name = format!("expr{}", *expr_counter);
                *expr_counter += 1;
                Some(name)
            }
        }
    }
}

fn result_columns(items: &[ReturnItem]) -> Vec<String> {
    let mut expr_counter = 0;
    items
        .iter()
        .filter_map(|item| column_name(item, &mut expr_counter))
        .collect()
}

fn project_row(
    items: &[ReturnItem],
    bindings: &Bindings,
    graph: &Graph<impl CypherNode, impl CypherEdge>,
    parameters: &Parameters,
) -> Result<Row, CypherError> {
    let mut values = HashMap::new();
    let mut expr_counter = 0;

    for item in items {
        let key = column_name(item, &mut expr_counter).unwrap_or_else(|| "*".to_string());

        if matches!(item.expression, Expression::All) {
            for (var, bound) in bindings {
                match bound {
                    BoundValue::Node(idx) => {
                        let data = &graph[*idx];
                        values.insert(
                            var.clone(),
                            ResultValue::Node {
                                labels: data.labels(),
                                properties: data.properties(),
                            },
                        );
                    }
                    BoundValue::Edge(idx) => {
                        let data = &graph[*idx];
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

        let result_value = project_expression(&item.expression, bindings, graph, parameters)?;
        values.insert(key, result_value);
    }

    Ok(Row { values })
}

fn compare_sort_keys(a: &CypherValue, b: &CypherValue) -> std::cmp::Ordering {
    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
}

fn stable_cypher_value_string(v: &CypherValue) -> String {
    match v {
        CypherValue::Null => "N".to_string(),
        CypherValue::Boolean(b) => format!("B{}", b),
        CypherValue::Integer(i) => format!("I{}", i),
        CypherValue::Float(f) => format!("F{}", f.to_bits()),
        CypherValue::String(s) => format!("S{}", s),
        CypherValue::List(items) => {
            let parts: Vec<_> = items.iter().map(stable_cypher_value_string).collect();
            format!("L[{}]", parts.join(","))
        }
        CypherValue::Map(entries) => {
            let mut keys: Vec<_> = entries.keys().collect();
            keys.sort();
            let parts: Vec<_> = keys
                .iter()
                .map(|k| format!("{}:{}", k, stable_cypher_value_string(&entries[*k])))
                .collect();
            format!("M{{{}}}", parts.join(","))
        }
    }
}

fn stable_result_value_string(v: &ResultValue) -> String {
    match v {
        ResultValue::Scalar(cv) => stable_cypher_value_string(cv),
        ResultValue::Node { labels, properties } => {
            let mut labels: Vec<_> = labels.clone();
            labels.sort();
            let mut keys: Vec<_> = properties.keys().collect();
            keys.sort();
            let parts: Vec<_> = keys
                .iter()
                .map(|k| format!("{}:{}", k, stable_cypher_value_string(&properties[*k])))
                .collect();
            format!("Node({}:{{{}}})", labels.join(":"), parts.join(","))
        }
        ResultValue::Edge {
            rel_type,
            properties,
        } => {
            let mut keys: Vec<_> = properties.keys().collect();
            keys.sort();
            let parts: Vec<_> = keys
                .iter()
                .map(|k| format!("{}:{}", k, stable_cypher_value_string(&properties[*k])))
                .collect();
            format!(
                "Edge({}:{{{}}})",
                rel_type.as_deref().unwrap_or(""),
                parts.join(",")
            )
        }
    }
}

fn row_canonical_key(row: &Row) -> String {
    let mut pairs: Vec<_> = row
        .values
        .iter()
        .map(|(k, v)| (k.clone(), stable_result_value_string(v)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut result = String::new();
    for (k, v) in pairs {
        result.push_str(&k);
        result.push('=');
        result.push_str(&v);
        result.push(';');
    }
    result
}

fn apply_distinct(rows: Vec<Row>) -> Vec<Row> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let key = row_canonical_key(&row);
        if seen.insert(key) {
            result.push(row);
        }
    }
    result
}

fn apply_skip_limit(rows: Vec<Row>, skip: Option<usize>, limit: Option<usize>) -> Vec<Row> {
    let skip = skip.unwrap_or(0);
    let mut rows: Vec<Row> = rows.into_iter().skip(skip).collect();
    if let Some(limit) = limit {
        rows.truncate(limit);
    }
    rows
}

// ---------------------------------------------------------------------------
// Read-only executor
// ---------------------------------------------------------------------------

struct ReadQueryExecutor<'g, 'p, N: CypherNode, E: CypherEdge> {
    graph: &'g Graph<N, E>,
    strategy: MatchStrategy,
    parameters: &'p Parameters,
}

impl<'g, 'p, N: CypherNode, E: CypherEdge> ReadQueryExecutor<'g, 'p, N, E> {
    fn new(graph: &'g Graph<N, E>, strategy: MatchStrategy, parameters: &'p Parameters) -> Self {
        Self {
            graph,
            strategy,
            parameters,
        }
    }

    fn execute(self, query: CypherQuery) -> Result<QueryResult<'g, N, E>, CypherError> {
        let mut bindings: Vec<Bindings> = vec![HashMap::new()];
        let mut return_items: Option<Vec<ReturnItem>> = None;
        let mut return_distinct = false;
        let mut order_by: Option<Vec<SortItem>> = None;
        let mut skip: Option<usize> = None;
        let mut limit: Option<usize> = None;

        for clause in query.clauses {
            match clause {
                Clause::Match {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        bindings.retain(|b| {
                            evaluate_where(&where_expr, b, self.graph, self.parameters)
                        });
                    }
                }
                Clause::OptionalMatch {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_optional_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        bindings.retain(|b| {
                            evaluate_where(&where_expr, b, self.graph, self.parameters)
                        });
                    }
                }
                Clause::Unwind {
                    expression,
                    variable,
                } => {
                    bindings = self.execute_unwind(expression, variable, bindings)?;
                }
                Clause::Return { items, distinct } => {
                    return_items = Some(items);
                    return_distinct = distinct;
                }
                Clause::OrderBy { items } => {
                    order_by = Some(items);
                }
                Clause::Skip { count } => {
                    skip = Some(count);
                }
                Clause::Limit { count } => {
                    limit = Some(count);
                }
                Clause::Create { .. }
                | Clause::Merge { .. }
                | Clause::Delete { .. }
                | Clause::Set { .. }
                | Clause::Remove { .. }
                | Clause::With { .. } => {
                    unreachable!("validated by validate_read_query")
                }
            }
        }

        // Apply ORDER BY to bindings before projection, so sort expressions
        // can reference variables that may not appear in the RETURN clause.
        if let Some(sort_items) = &order_by {
            let mut sort_keys: Vec<Vec<CypherValue>> = Vec::with_capacity(bindings.len());
            for binding in &bindings {
                let mut keys = Vec::with_capacity(sort_items.len());
                for item in sort_items {
                    let val = evaluate_expression(
                        &item.expression,
                        binding,
                        self.graph,
                        self.parameters,
                    )?;
                    keys.push(val);
                }
                sort_keys.push(keys);
            }
            let mut indexed: Vec<_> = bindings.into_iter().zip(sort_keys).collect();
            indexed.sort_by(|(_, a_keys), (_, b_keys)| {
                for ((a_val, b_val), item) in
                    a_keys.iter().zip(b_keys.iter()).zip(sort_items.iter())
                {
                    let cmp = compare_sort_keys(a_val, b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return match item.direction {
                            SortDirection::Asc => cmp,
                            SortDirection::Desc => cmp.reverse(),
                        };
                    }
                }
                std::cmp::Ordering::Equal
            });
            bindings = indexed.into_iter().map(|(b, _)| b).collect();
        }

        let mut rows = Vec::new();
        if let Some(items) = return_items {
            let columns = result_columns(&items);
            for binding in bindings {
                let row = project_row(&items, &binding, self.graph, self.parameters)?;
                rows.push(row);
            }

            if return_distinct {
                rows = apply_distinct(rows);
            }

            rows = apply_skip_limit(rows, skip, limit);

            return Ok(QueryResult::new(columns, rows, self.graph));
        }

        Ok(QueryResult::new(vec![], rows, self.graph))
    }

    fn execute_match(
        &self,
        patterns: Vec<PathPattern>,
        input_bindings: Vec<Bindings>,
    ) -> Vec<Bindings> {
        if patterns.is_empty() {
            return input_bindings;
        }

        let matcher = PatternMatcher::new(self.graph, self.strategy);
        let mut results = Vec::new();
        for existing_bindings in &input_bindings {
            let pattern_results = matcher.match_patterns(&patterns, existing_bindings);
            results.extend(pattern_results);
        }
        results
    }

    fn execute_optional_match(
        &self,
        patterns: Vec<PathPattern>,
        input_bindings: Vec<Bindings>,
    ) -> Vec<Bindings> {
        if patterns.is_empty() {
            return input_bindings;
        }

        let matcher = PatternMatcher::new(self.graph, self.strategy);
        let mut results = Vec::new();
        for existing_bindings in &input_bindings {
            let pattern_results = matcher.match_patterns(&patterns, existing_bindings);
            if pattern_results.is_empty() {
                // Missing bindings already behave like NULL in the evaluator/projector,
                // so we just propagate the existing bindings without additions.
                results.push(existing_bindings.clone());
            } else {
                results.extend(pattern_results);
            }
        }
        results
    }

    fn execute_unwind(
        &self,
        _expression: Expression,
        _variable: String,
        _input_bindings: Vec<Bindings>,
    ) -> Result<Vec<Bindings>, CypherError> {
        Err(CypherError::Unsupported(
            "UNWIND is not yet fully supported".into(),
        ))
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
        let no_parameters = Parameters::new();

        for clause in query.clauses {
            match clause {
                Clause::Match {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        bindings
                            .retain(|b| evaluate_where(&where_expr, b, self.graph, &no_parameters));
                    }
                }
                Clause::OptionalMatch {
                    patterns,
                    where_clause,
                } => {
                    bindings = self.execute_optional_match(patterns, bindings);
                    if let Some(where_expr) = where_clause {
                        bindings
                            .retain(|b| evaluate_where(&where_expr, b, self.graph, &no_parameters));
                    }
                }
                Clause::Create { patterns } => {
                    for bindings_row in &mut bindings {
                        for pattern in &patterns {
                            self.apply_path_pattern_mut(bindings_row, pattern)?;
                        }
                    }
                }
                Clause::Merge {
                    pattern,
                    on_create,
                    on_match,
                } => {
                    for bindings_row in &mut bindings {
                        let matcher = PatternMatcher::new(self.graph, MatchStrategy::Backtrack);
                        let test_results = matcher.match_single_path(&pattern, bindings_row);
                        if test_results.is_empty() {
                            self.apply_path_pattern_mut(bindings_row, &pattern)?;
                            self.apply_set_items(bindings_row, &on_create, &no_parameters)?;
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
                            self.apply_set_items(bindings_row, &on_match, &no_parameters)?;
                        }
                    }
                }
                Clause::Delete { variables, detach } => {
                    self.execute_delete(&variables, detach, &bindings)?;
                }
                Clause::Set { items } => {
                    for bindings_row in &bindings {
                        self.apply_set_items(bindings_row, &items, &no_parameters)?;
                    }
                }
                Clause::Remove { items } => {
                    for bindings_row in &bindings {
                        self.apply_remove_items(bindings_row, &items)?;
                    }
                }
                Clause::Return { .. } => {
                    unreachable!("validated by validate_mutation_query")
                }
                Clause::OrderBy { .. }
                | Clause::Skip { .. }
                | Clause::Limit { .. }
                | Clause::Unwind { .. }
                | Clause::With { .. } => {
                    return Err(CypherError::Unsupported(format!(
                        "clause {:?} is not supported in mutation queries",
                        std::mem::discriminant(&clause)
                    )));
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

    fn execute_optional_match(
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
            if pattern_results.is_empty() {
                results.push(existing_bindings.clone());
            } else {
                results.extend(pattern_results);
            }
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

    fn apply_set_items(
        &mut self,
        bindings: &Bindings,
        items: &[SetItem],
        parameters: &Parameters,
    ) -> Result<(), CypherError> {
        for item in items {
            match item {
                SetItem::SetProperty {
                    variable,
                    property,
                    value,
                } => {
                    if let Some(bound) = bindings.get(variable) {
                        let val = evaluate_expression(value, bindings, self.graph, parameters)?;
                        match bound {
                            BoundValue::Node(idx) => {
                                if matches!(val, CypherValue::Null) {
                                    self.graph[*idx].properties.remove(property);
                                } else {
                                    self.graph[*idx].properties.insert(property.clone(), val);
                                }
                            }
                            BoundValue::Edge(idx) => {
                                if matches!(val, CypherValue::Null) {
                                    self.graph[*idx].properties.remove(property);
                                } else {
                                    self.graph[*idx].properties.insert(property.clone(), val);
                                }
                            }
                        }
                    }
                }
                SetItem::SetVariable {
                    variable,
                    properties,
                } => {
                    if let Some(bound) = bindings.get(variable) {
                        match bound {
                            BoundValue::Node(idx) => {
                                self.graph[*idx].properties = properties.clone();
                            }
                            BoundValue::Edge(idx) => {
                                self.graph[*idx].properties = properties.clone();
                            }
                        }
                    }
                }
                SetItem::SetLabels { variable, labels } => {
                    if let Some(BoundValue::Node(idx)) = bindings.get(variable) {
                        let node_data = &mut self.graph[*idx];
                        for label in labels {
                            if !node_data.has_label(label) {
                                node_data.labels.push(label.clone());
                            }
                        }
                    }
                }
                SetItem::MergeProperties {
                    variable,
                    properties,
                } => {
                    if let Some(bound) = bindings.get(variable) {
                        match bound {
                            BoundValue::Node(idx) => {
                                for (k, v) in properties {
                                    self.graph[*idx].properties.insert(k.clone(), v.clone());
                                }
                            }
                            BoundValue::Edge(idx) => {
                                for (k, v) in properties {
                                    self.graph[*idx].properties.insert(k.clone(), v.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_remove_items(
        &mut self,
        bindings: &Bindings,
        items: &[RemoveItem],
    ) -> Result<(), CypherError> {
        for item in items {
            match item {
                RemoveItem::RemoveProperty { variable, property } => {
                    if let Some(bound) = bindings.get(variable) {
                        match bound {
                            BoundValue::Node(idx) => {
                                self.graph[*idx].properties.remove(property);
                            }
                            BoundValue::Edge(idx) => {
                                self.graph[*idx].properties.remove(property);
                            }
                        }
                    }
                }
                RemoveItem::RemoveLabels { variable, labels } => {
                    if let Some(BoundValue::Node(idx)) = bindings.get(variable) {
                        let node_data = &mut self.graph[*idx];
                        node_data.labels.retain(|l| !labels.contains(l));
                    }
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
        if let Some(var) = &pattern.variable
            && let Some(&existing) = var_map.get(var)
        {
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
