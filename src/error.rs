//! Error types for petgraph-cypher.

use thiserror::Error;

/// Errors that can occur when parsing or evaluating a Cypher query.
#[derive(Debug, Error, PartialEq)]
pub enum CypherError {
    /// The query string could not be parsed.
    #[error("parse error: {0}")]
    ParseError(String),

    /// A feature required by the query is not yet implemented.
    #[error("unsupported clause or feature: {0}")]
    Unsupported(String),

    /// The query is structurally invalid (e.g. an unresolved variable).
    #[error("invalid query: {0}")]
    InvalidQuery(String),
}
