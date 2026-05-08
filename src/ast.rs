//! Abstract Syntax Tree types for OpenCypher queries.

use std::collections::HashMap;

/// A literal value in a Cypher property map or expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CypherValue {
    /// A UTF-8 string literal.
    String(String),
    /// A 64-bit signed integer literal.
    Integer(i64),
    /// A 64-bit floating-point literal.
    Float(f64),
    /// A boolean literal (`true` / `false`).
    Boolean(bool),
    /// The `null` literal.
    Null,
}

impl std::fmt::Display for CypherValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CypherValue::String(s) => write!(f, "\"{}\"", s),
            CypherValue::Integer(i) => write!(f, "{}", i),
            CypherValue::Float(v) => write!(f, "{}", v),
            CypherValue::Boolean(b) => write!(f, "{}", b),
            CypherValue::Null => write!(f, "null"),
        }
    }
}

/// A node pattern such as `(n:Person {name: "Alice"})`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    /// Optional bound variable name (e.g. `n`).
    pub variable: Option<String>,
    /// Zero or more node labels (e.g. `["Person"]`).
    pub labels: Vec<String>,
    /// Inline property map (e.g. `{name: "Alice"}`).
    pub properties: HashMap<String, CypherValue>,
}

/// The direction of a relationship pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelDirection {
    /// `-[...]->`  – relationship points right (source → target).
    Right,
    /// `<-[...]-`  – relationship points left (target ← source).
    Left,
    /// `-[...]-`   – relationship is undirected / direction-agnostic.
    Both,
}

/// A relationship pattern such as `-[:KNOWS {since: 2020}]->`.
#[derive(Debug, Clone, PartialEq)]
pub struct RelPattern {
    /// Optional bound variable name (e.g. `r`).
    pub variable: Option<String>,
    /// Optional relationship type (e.g. `"KNOWS"`).
    pub rel_type: Option<String>,
    /// Inline property map.
    pub properties: HashMap<String, CypherValue>,
    /// Direction of the relationship.
    pub direction: RelDirection,
}

/// A path pattern: a start node followed by zero or more
/// (relationship, node) pairs.
///
/// Example: `(a)-[:KNOWS]->(b)-[:LIKES]->(c)`
#[derive(Debug, Clone, PartialEq)]
pub struct PathPattern {
    /// The first node in the path.
    pub start: NodePattern,
    /// Each subsequent hop: `(relationship_pattern, target_node_pattern)`.
    pub rels: Vec<(RelPattern, NodePattern)>,
}

/// An item in a `RETURN` clause.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    /// The expression being returned.
    pub expression: Expression,
    /// An optional `AS alias` name.
    pub alias: Option<String>,
}

/// An expression that can appear in a `RETURN` clause or `WHERE` condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// A bare variable reference (e.g. `n`).
    Variable(String),
    /// A property access (e.g. `n.name`).
    Property(String, String),
    /// The wildcard `*` (return everything).
    All,
}

/// A basic `WHERE` expression (subset of OpenCypher).
#[derive(Debug, Clone, PartialEq)]
pub enum WhereExpr {
    /// An equality check: `n.prop = value`.
    Eq(Expression, CypherValue),
    /// Logical AND of two sub-expressions.
    And(Box<WhereExpr>, Box<WhereExpr>),
    /// Logical OR of two sub-expressions.
    Or(Box<WhereExpr>, Box<WhereExpr>),
}

/// A single Cypher clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    /// `MATCH pattern [WHERE condition]`
    Match {
        patterns: Vec<PathPattern>,
        where_clause: Option<WhereExpr>,
    },
    /// `CREATE pattern`
    Create { patterns: Vec<PathPattern> },
    /// `MERGE pattern`
    Merge { pattern: PathPattern },
    /// `RETURN items`
    Return { items: Vec<ReturnItem> },
    /// `[DETACH] DELETE variables`
    Delete {
        variables: Vec<String>,
        detach: bool,
    },
}

/// A parsed OpenCypher query consisting of one or more clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherQuery {
    /// The ordered list of clauses in this query.
    pub clauses: Vec<Clause>,
}
