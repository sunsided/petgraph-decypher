//! Error types for petgraph-decypher.

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

    /// A runtime type mismatch occurred during expression evaluation.
    #[error("type mismatch: {0}")]
    TypeMismatch(String),

    /// Division by zero during expression evaluation.
    #[error("division by zero")]
    DivisionByZero,

    /// An invalid operation was requested at runtime.
    #[error("runtime error: {0}")]
    RuntimeError(String),
}
