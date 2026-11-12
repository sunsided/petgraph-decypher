//! `petgraph-decypher` – run openCypher queries against [`petgraph`] graphs.
//!
//! Queries are parsed and analyzed by [`decypher`] and executed directly on
//! its high-level IR. Read queries work on any `Graph<N, E>` or
//! `StableGraph<N, E>` whose weights implement [`CypherNode`] /
//! [`CypherEdge`]; mutating queries also need [`CypherNodeMut`] /
//! [`CypherEdgeMut`]. [`NodeData`] and [`EdgeData`] implement all four.
//!
//! # Example
//!
//! ```rust
//! use petgraph_decypher::{PetgraphCypher, PetgraphCypherMut, Value, build_graph_from_cypher};
//!
//! let mut graph = build_graph_from_cypher(
//!     "CREATE (:Person {name: 'Alice'})-[:KNOWS]->(:Person {name: 'Bob'})",
//! )?;
//! graph.cypher_mut("MATCH (p:Person {name: 'Bob'}) SET p.age = 42")?;
//!
//! let result = graph.cypher("MATCH (a)-[:KNOWS]->(b) RETURN a.name, b.age")?;
//! assert_eq!(result.columns(), ["a.name", "b.age"]);
//! let row = result.rows().next().unwrap();
//! assert_eq!(row.get("a.name"), Some(&Value::from("Alice")));
//! assert_eq!(row.get("b.age"), Some(&Value::from(42)));
//! # Ok::<(), petgraph_decypher::CypherError>(())
//! ```
//!
//! Use [`Query`] to parse a query once and run it many times.
//!
//! # Graph types and deletion
//!
//! `DELETE` takes effect when the query finishes. On a [`petgraph::Graph`],
//! removing an element moves the last element into its slot, so indices of
//! *other* elements may change; use a [`petgraph::stable_graph::StableGraph`]
//! when indices must stay valid.

mod element;
pub mod error;
mod exec;
mod graph;
mod prepared;
mod result;
mod value;

pub use element::{CypherEdge, CypherEdgeMut, CypherNode, CypherNodeMut, EdgeData, NodeData};
pub use error::CypherError;
pub use exec::Parameters;
pub use graph::{CypherGraph, CypherGraphMut};
pub use prepared::Query;
pub use result::{QueryResult, Row};
pub use value::{Path, Properties, Value};

use petgraph::Graph;

/// Runs read-only Cypher queries on a graph.
pub trait PetgraphCypher: CypherGraph + Sized {
    /// Runs a read-only query.
    fn cypher(&self, query: &str) -> Result<QueryResult, CypherError> {
        Query::new(query)?.execute(self, &Parameters::new())
    }

    /// Runs a read-only query with parameters (`$name` reads `params["name"]`).
    fn cypher_params(&self, query: &str, params: &Parameters) -> Result<QueryResult, CypherError> {
        Query::new(query)?.execute(self, params)
    }
}

impl<G: CypherGraph> PetgraphCypher for G {}

/// Runs any Cypher query, including `CREATE`, `MERGE`, `SET`, `REMOVE` and
/// `DELETE`, on a graph.
pub trait PetgraphCypherMut: CypherGraphMut + Sized {
    /// Runs a query that may modify the graph and returns its rows.
    fn cypher_mut(&mut self, query: &str) -> Result<QueryResult, CypherError> {
        Query::new(query)?.execute_mut(self, &Parameters::new())
    }

    /// Runs a query that may modify the graph, with parameters.
    fn cypher_mut_params(
        &mut self,
        query: &str,
        params: &Parameters,
    ) -> Result<QueryResult, CypherError> {
        Query::new(query)?.execute_mut(self, params)
    }
}

impl<G: CypherGraphMut> PetgraphCypherMut for G {}

/// Builds a new graph with custom weights by running `query` (typically
/// `CREATE` / `MERGE` clauses) on an empty graph.
///
/// ```rust
/// use petgraph_decypher::{EdgeData, NodeData, build_graph_from_cypher_typed};
///
/// let graph = build_graph_from_cypher_typed::<NodeData, EdgeData>(
///     "CREATE (:Person {name: 'Alice'})-[:KNOWS]->(:Person {name: 'Bob'})",
/// )?;
/// assert_eq!((graph.node_count(), graph.edge_count()), (2, 1));
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
pub fn build_graph_from_cypher_typed<N: CypherNodeMut, E: CypherEdgeMut>(
    query: &str,
) -> Result<Graph<N, E>, CypherError> {
    let mut graph = Graph::new();
    graph.cypher_mut(query)?;
    Ok(graph)
}

/// Builds a new `Graph<NodeData, EdgeData>` by running `query` on an empty
/// graph.
///
/// ```rust
/// use petgraph_decypher::build_graph_from_cypher;
///
/// let graph = build_graph_from_cypher(
///     "CREATE (a:Person {name: 'Alice'})-[:KNOWS]->(b:Person {name: 'Bob'})",
/// )?;
/// let edge = graph.edge_indices().next().unwrap();
/// assert_eq!(graph[edge].rel_type, "KNOWS");
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
pub fn build_graph_from_cypher(query: &str) -> Result<Graph<NodeData, EdgeData>, CypherError> {
    build_graph_from_cypher_typed(query)
}
