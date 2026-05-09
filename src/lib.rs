//! `petgraph-decypher` – build [`petgraph`] graphs from OpenCypher queries.
//!
//! # Overview
//!
//! This crate provides three main entry points:
//!
//! * [`parse_cypher`] – parse a Cypher query string into the internal query plan.
//! * [`build_graph_from_cypher`] – parse a Cypher query and materialise all
//!   `CREATE` / `MERGE` operations into a [`petgraph::Graph`] with the default
//!   [`NodeData`] / [`EdgeData`] types.
//! * [`build_graph_from_cypher_typed`] – generic version that accepts any types
//!   implementing [`CypherNode`] and [`CypherEdge`].
//!
//! # Supported Cypher subset
//!
//! | Feature | Status |
//! |---------|--------|
//! | `CREATE (n:Label {k: v})-[:TYPE]->(m)` | ✅ materialised + executed |
//! | `MERGE  (n:Label {k: v})-[:TYPE]->(m)` | ✅ materialised + executed |
//! | `MATCH  (n)-[r]->(m) WHERE n.p = v`    | ✅ evaluated |
//! | `RETURN n, n.prop AS alias` / `RETURN *` | ✅ evaluated |
//! | `[DETACH] DELETE n`                    | ✅ executed |
//! | Multiple clauses in one query          | ✅ |
//! | Semicolon-separated statements         | ✅ |
//!
//! # Example
//!
//! ```rust
//! use petgraph_decypher::build_graph_from_cypher;
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
mod planner;
pub mod query;

pub use ast::{
    BinaryOp, CaseExpr, Clause, CypherQuery, CypherValue, Expression, NodePattern, PathPattern,
    RelDirection, RelPattern, RelationshipLength, ReturnItem, SetItem, SortDirection, SortItem,
    UnaryOp, WhereExpr,
};
pub use error::CypherError;
pub use query::{MatchStrategy, PetgraphCypher, PetgraphCypherRead, QueryResult, ResultValue, Row};

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

/// Common property access used by the query planner and executor.
pub trait CypherProperties {
    /// Get a property value by name.
    fn get(&self, name: &str) -> Option<&CypherValue>;

    /// Return all properties as a cloned map for query results.
    fn properties(&self) -> HashMap<String, CypherValue>;
}

/// Metadata required from node weights during Cypher planning and execution.
pub trait CypherNode: CypherProperties {
    /// Returns whether this node has the requested label.
    fn has_label(&self, label: &str) -> bool;

    /// Return all node labels.
    fn labels(&self) -> Vec<String>;

    /// Construct a node value from a Cypher pattern.
    fn from_cypher(
        variable: Option<String>,
        labels: Vec<String>,
        properties: HashMap<String, CypherValue>,
    ) -> Self;
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

/// Metadata required from edge weights during Cypher planning and execution.
pub trait CypherEdge: CypherProperties {
    /// Returns whether this edge matches the requested relationship type.
    fn has_rel_type(&self, rel_type: &str) -> bool;

    /// Return the relationship type, if any.
    fn rel_type(&self) -> Option<&str>;

    /// Construct an edge value from a Cypher pattern.
    fn from_cypher(
        variable: Option<String>,
        rel_type: Option<String>,
        properties: HashMap<String, CypherValue>,
    ) -> Self;
}

impl CypherProperties for NodeData {
    fn get(&self, name: &str) -> Option<&CypherValue> {
        self.properties.get(name)
    }

    fn properties(&self) -> HashMap<String, CypherValue> {
        self.properties.clone()
    }
}

impl CypherNode for NodeData {
    fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|node_label| node_label == label)
    }

    fn labels(&self) -> Vec<String> {
        self.labels.clone()
    }

    fn from_cypher(
        variable: Option<String>,
        labels: Vec<String>,
        properties: HashMap<String, CypherValue>,
    ) -> Self {
        Self {
            variable,
            labels,
            properties,
        }
    }
}

impl CypherProperties for EdgeData {
    fn get(&self, name: &str) -> Option<&CypherValue> {
        self.properties.get(name)
    }

    fn properties(&self) -> HashMap<String, CypherValue> {
        self.properties.clone()
    }
}

impl CypherEdge for EdgeData {
    fn has_rel_type(&self, rel_type: &str) -> bool {
        self.rel_type.as_deref() == Some(rel_type)
    }

    fn rel_type(&self) -> Option<&str> {
        self.rel_type.as_deref()
    }

    fn from_cypher(
        variable: Option<String>,
        rel_type: Option<String>,
        properties: HashMap<String, CypherValue>,
    ) -> Self {
        Self {
            variable,
            rel_type,
            properties,
        }
    }
}

/// Parse a Cypher query string and return its HIR-backed query plan representation.
///
/// # Errors
///
/// Returns [`CypherError::ParseError`] if the input cannot be parsed.
///
/// # Example
///
/// ```rust
/// use petgraph_decypher::parse_cypher;
///
/// let query = parse_cypher("CREATE (n:Person {name: \"Alice\"})").unwrap();
/// assert_eq!(query.clauses.len(), 1);
/// ```
pub fn parse_cypher(query: &str) -> Result<CypherQuery, CypherError> {
    planner::plan_query(query)
}

/// Parse a Cypher query and build a petgraph [`Graph`] from its `CREATE` and
/// `MERGE` clauses using custom node and edge types.
///
/// This is the generic version of [`build_graph_from_cypher`]. Use it when you
/// want to supply your own types that implement [`CypherNode`] and [`CypherEdge`].
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
/// use petgraph_decypher::{build_graph_from_cypher_typed, NodeData, EdgeData};
///
/// let graph = build_graph_from_cypher_typed::<NodeData, EdgeData>(
///     r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
/// )
/// .unwrap();
///
/// assert_eq!(graph.node_count(), 2);
/// assert_eq!(graph.edge_count(), 1);
/// ```
pub fn build_graph_from_cypher_typed<N: CypherNode, E: CypherEdge>(
    query: &str,
) -> Result<Graph<N, E>, CypherError> {
    let ast = parse_cypher(query)?;
    builder::build_graph(ast)
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
/// use petgraph_decypher::{build_graph_from_cypher, CypherValue};
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
    build_graph_from_cypher_typed(query)
}
