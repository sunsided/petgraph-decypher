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
    /// A list of values.
    List(Vec<CypherValue>),
    /// A map of key-value pairs.
    Map(HashMap<String, CypherValue>),
}

impl std::fmt::Display for CypherValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CypherValue::String(s) => write!(f, "\"{}\"", s),
            CypherValue::Integer(i) => write!(f, "{}", i),
            CypherValue::Float(v) => write!(f, "{}", v),
            CypherValue::Boolean(b) => write!(f, "{}", b),
            CypherValue::Null => write!(f, "null"),
            CypherValue::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            CypherValue::Map(entries) => {
                write!(f, "{{")?;
                let mut keys: Vec<_> = entries.keys().collect();
                keys.sort();
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, entries.get(*k).unwrap())?;
                }
                write!(f, "}}")
            }
        }
    }
}

impl std::cmp::PartialOrd for CypherValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        match (self, other) {
            (CypherValue::Null, CypherValue::Null) => Some(Ordering::Equal),
            (CypherValue::Null, _) => Some(Ordering::Less),
            (_, CypherValue::Null) => Some(Ordering::Greater),
            (CypherValue::Boolean(a), CypherValue::Boolean(b)) => a.partial_cmp(b),
            (CypherValue::Integer(a), CypherValue::Integer(b)) => a.partial_cmp(b),
            (CypherValue::Integer(a), CypherValue::Float(b)) => (*a as f64).partial_cmp(b),
            (CypherValue::Float(a), CypherValue::Integer(b)) => a.partial_cmp(&(*b as f64)),
            (CypherValue::Float(a), CypherValue::Float(b)) => a.partial_cmp(b),
            (CypherValue::String(a), CypherValue::String(b)) => a.partial_cmp(b),
            (CypherValue::List(a), CypherValue::List(b)) => a.partial_cmp(b),
            (CypherValue::Map(_), CypherValue::Map(_)) => None,
            // Cross-type ordering (arbitrary but consistent)
            (CypherValue::Boolean(_), _) => Some(Ordering::Less),
            (_, CypherValue::Boolean(_)) => Some(Ordering::Greater),
            (CypherValue::Integer(_), _) => Some(Ordering::Less),
            (_, CypherValue::Integer(_)) => Some(Ordering::Greater),
            (CypherValue::Float(_), _) => Some(Ordering::Less),
            (_, CypherValue::Float(_)) => Some(Ordering::Greater),
            (CypherValue::String(_), _) => Some(Ordering::Less),
            (_, CypherValue::String(_)) => Some(Ordering::Greater),
            (CypherValue::List(_), _) => Some(Ordering::Less),
            (_, CypherValue::List(_)) => Some(Ordering::Greater),
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
    /// Variable-length relationship bounds, if specified.
    pub length: Option<RelationshipLength>,
}

/// Length bounds for variable-length relationships.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipLength {
    /// Minimum hops (inclusive). `None` means 1 for `*n..m`, 0 for `*`.
    pub min: Option<usize>,
    /// Maximum hops (inclusive). `None` means unbounded.
    pub max: Option<usize>,
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

/// An item in a `RETURN` or `WITH` clause.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnItem {
    /// The expression being returned.
    pub expression: Expression,
    /// An optional `AS alias` name.
    pub alias: Option<String>,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Logical negation (`NOT`).
    Not,
    /// Arithmetic negation (`-`).
    Negate,
    /// Unary plus (`+`, no-op).
    Plus,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `%`
    Modulo,
    /// `^`
    Power,
    /// `=`
    Eq,
    /// `<>` / `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `AND`
    And,
    /// `OR`
    Or,
    /// `XOR`
    Xor,
    /// `STARTS WITH`
    StartsWith,
    /// `ENDS WITH`
    EndsWith,
    /// `CONTAINS`
    Contains,
    /// `IN`
    In,
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
    /// A literal value.
    Literal(CypherValue),
    /// A unary operation.
    Unary(UnaryOp, Box<Expression>),
    /// A binary infix operation.
    Binary(BinaryOp, Box<Expression>, Box<Expression>),
    /// A list literal `[e1, e2, …]`.
    List(Vec<Expression>),
    /// A function or aggregate call.
    FunctionCall {
        /// Function name.
        name: String,
        /// Positional arguments.
        args: Vec<Expression>,
        /// `true` when called as `f(DISTINCT …)`.
        distinct: bool,
    },
    /// A parameter reference (`$name`).
    Parameter(String),
    /// A `CASE … END` expression.
    Case(CaseExpr),
}

/// A `CASE … END` expression.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseExpr {
    /// The scrutinee for generic `CASE expr WHEN …`, or `None` for searched form.
    pub scrutinee: Option<Box<Expression>>,
    /// The `WHEN … THEN …` alternatives.
    pub alternatives: Vec<(Expression, Expression)>,
    /// The `ELSE` default, or `None`.
    pub default: Option<Box<Expression>>,
}

/// A basic `WHERE` expression (subset of OpenCypher).
#[derive(Debug, Clone, PartialEq)]
pub enum WhereExpr {
    /// An equality check: `n.prop = value`.
    Eq(Expression, Expression),
    /// A not-equal check: `n.prop <> value`.
    NotEq(Expression, Expression),
    /// Less than: `n.prop < value`.
    Lt(Expression, Expression),
    /// Greater than: `n.prop > value`.
    Gt(Expression, Expression),
    /// Less than or equal: `n.prop <= value`.
    Le(Expression, Expression),
    /// Greater than or equal: `n.prop >= value`.
    Ge(Expression, Expression),
    /// List membership: `n.prop IN […]`.
    In(Expression, Vec<CypherValue>),
    /// String starts with: `n.prop STARTS WITH "prefix"`.
    StartsWith(Expression, String),
    /// String ends with: `n.prop ENDS WITH "suffix"`.
    EndsWith(Expression, String),
    /// String contains: `n.prop CONTAINS "substr"`.
    Contains(Expression, String),
    /// `IS NULL` check.
    IsNull(Expression),
    /// `IS NOT NULL` check.
    IsNotNull(Expression),
    /// Logical NOT of a sub-expression.
    Not(Box<WhereExpr>),
    /// Logical AND of two sub-expressions.
    And(Box<WhereExpr>, Box<WhereExpr>),
    /// Logical OR of two sub-expressions.
    Or(Box<WhereExpr>, Box<WhereExpr>),
    /// Logical XOR of two sub-expressions.
    Xor(Box<WhereExpr>, Box<WhereExpr>),
}

/// Sort direction for `ORDER BY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// A single `ORDER BY` criterion.
#[derive(Debug, Clone, PartialEq)]
pub struct SortItem {
    /// The expression to sort by.
    pub expression: Expression,
    /// The sort direction.
    pub direction: SortDirection,
}

/// An item in a `SET` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum SetItem {
    /// `variable.property = value`
    SetProperty {
        variable: String,
        property: String,
        value: Expression,
    },
    /// `variable = map` (replace all properties)
    SetVariable {
        variable: String,
        properties: HashMap<String, CypherValue>,
    },
    /// `variable:Label1:Label2` — add labels.
    SetLabels {
        variable: String,
        labels: Vec<String>,
    },
    /// `variable += map` (merge properties)
    MergeProperties {
        variable: String,
        properties: HashMap<String, CypherValue>,
    },
}

/// An item in a `REMOVE` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoveItem {
    /// Remove a property.
    RemoveProperty { variable: String, property: String },
    /// Remove labels from a node.
    RemoveLabels {
        variable: String,
        labels: Vec<String>,
    },
}

/// A single Cypher clause.
#[derive(Debug, Clone, PartialEq)]
pub enum Clause {
    /// `MATCH pattern [WHERE condition]`
    Match {
        patterns: Vec<PathPattern>,
        where_clause: Option<WhereExpr>,
    },
    /// `OPTIONAL MATCH pattern [WHERE condition]`
    OptionalMatch {
        patterns: Vec<PathPattern>,
        where_clause: Option<WhereExpr>,
    },
    /// `CREATE pattern`
    Create { patterns: Vec<PathPattern> },
    /// `MERGE pattern [ON CREATE SET …] [ON MATCH SET …]`
    Merge {
        pattern: PathPattern,
        on_create: Vec<SetItem>,
        on_match: Vec<SetItem>,
    },
    /// `RETURN items`
    Return {
        items: Vec<ReturnItem>,
        distinct: bool,
    },
    /// `WITH items`
    With {
        items: Vec<ReturnItem>,
        where_clause: Option<WhereExpr>,
        order_by: Option<Vec<SortItem>>,
        skip: Option<usize>,
        limit: Option<usize>,
        distinct: bool,
    },
    /// `UNWIND list AS variable`
    Unwind {
        expression: Expression,
        variable: String,
    },
    /// `[DETACH] DELETE variables`
    Delete {
        variables: Vec<String>,
        detach: bool,
    },
    /// `SET items`
    Set { items: Vec<SetItem> },
    /// `REMOVE items`
    Remove { items: Vec<RemoveItem> },
    /// `ORDER BY items`
    OrderBy { items: Vec<SortItem> },
    /// `SKIP n`
    Skip { count: usize },
    /// `LIMIT n`
    Limit { count: usize },
}

/// A parsed OpenCypher query consisting of one or more clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherQuery {
    /// The ordered list of clauses in this query.
    pub clauses: Vec<Clause>,
}
