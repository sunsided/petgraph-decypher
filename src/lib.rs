//! `petgraph-cypher` – build [`petgraph`] graphs from OpenCypher queries.
//!
//! # Overview
//!
//! This crate provides two main entry points:
//!
//! * [`parse_cypher`] – parse a Cypher query string and return the AST.
//! * [`build_graph_from_cypher`] – parse a Cypher query and materialise all
//!   `CREATE` / `MERGE` operations into a [`petgraph::Graph`].
//!
//! # Supported Cypher subset
//!
//! | Feature | Status |
//! |---------|--------|
//! | `CREATE (n:Label {k: v})-[:TYPE]->(m)` | ✅ |
//! | `MERGE  (n:Label {k: v})-[:TYPE]->(m)` | ✅ |
//! | `MATCH  (n)-[r]->(m) WHERE n.p = v`    | parsed, not evaluated |
//! | `RETURN n, n.prop AS alias, *`         | parsed |
//! | `[DETACH] DELETE n`                    | parsed |
//! | Multiple clauses in one query          | ✅ |
//! | Semicolon-separated statements         | ✅ |
//!
//! # Example
//!
//! ```rust
//! use petgraph_cypher::build_graph_from_cypher;
//!
//! let graph = build_graph_from_cypher(
//!     r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
//! )
//! .unwrap();
//!
//! assert_eq!(graph.node_count(), 2);
//! assert_eq!(graph.edge_count(), 1);
//! ```

pub mod ast;
mod builder;
pub mod error;
mod parser;
pub mod query;

pub use ast::{
    Clause, CypherQuery, CypherValue, Expression, NodePattern, PathPattern, RelDirection,
    RelPattern, ReturnItem, WhereExpr,
};
pub use error::CypherError;
pub use query::{MatchStrategy, PetgraphCypher, QueryResult, ResultValue, Row};

use petgraph::Graph;
use std::collections::HashMap;

/// Data stored at each node in the graph built by [`build_graph_from_cypher`].
#[derive(Debug, Clone, PartialEq)]
pub struct NodeData {
    /// Bound variable name from the Cypher pattern (e.g. `"n"`), if present.
    pub variable: Option<String>,
    /// Node labels (e.g. `["Person", "Employee"]`).
    pub labels: Vec<String>,
    /// Properties specified in the node pattern.
    pub properties: HashMap<String, CypherValue>,
}

/// Data stored at each edge in the graph built by [`build_graph_from_cypher`].
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeData {
    /// Bound variable name from the Cypher pattern (e.g. `"r"`), if present.
    pub variable: Option<String>,
    /// Relationship type (e.g. `"KNOWS"`), if specified.
    pub rel_type: Option<String>,
    /// Properties specified in the relationship pattern.
    pub properties: HashMap<String, CypherValue>,
}

/// Parse a Cypher query string and return its AST representation.
///
/// # Errors
///
/// Returns [`CypherError::ParseError`] if the input cannot be parsed.
///
/// # Example
///
/// ```rust
/// use petgraph_cypher::parse_cypher;
///
/// let query = parse_cypher("CREATE (n:Person {name: \"Alice\"})").unwrap();
/// assert_eq!(query.clauses.len(), 1);
/// ```
pub fn parse_cypher(query: &str) -> Result<CypherQuery, CypherError> {
    parser::parse_query(query)
}

/// Parse a Cypher query and build a petgraph [`Graph`] from its `CREATE` and
/// `MERGE` clauses.
///
/// Each distinct variable in the query corresponds to a single node; if the
/// same variable appears in multiple patterns it maps to the same
/// [`petgraph::graph::NodeIndex`].
///
/// # Errors
///
/// Returns [`CypherError::ParseError`] if the input cannot be parsed.
///
/// # Example
///
/// ```rust
/// use petgraph_cypher::{build_graph_from_cypher, CypherValue};
///
/// let graph = build_graph_from_cypher(
///     r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
/// )
/// .unwrap();
///
/// assert_eq!(graph.node_count(), 2);
/// assert_eq!(graph.edge_count(), 1);
///
/// // Check the edge type
/// let edge = graph.edge_indices().next().unwrap();
/// assert_eq!(graph[edge].rel_type.as_deref(), Some("KNOWS"));
/// ```
pub fn build_graph_from_cypher(query: &str) -> Result<Graph<NodeData, EdgeData>, CypherError> {
    let ast = parse_cypher(query)?;
    builder::build_graph(ast)
}
