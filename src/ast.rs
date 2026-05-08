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
    /// Relationship types (e.g. `["KNOWS"]` or `["KNOWS", "FRIENDS"]`).
    /// Empty means match any type.
    pub rel_types: Vec<String>,
    /// Inline property map.
    pub properties: HashMap<String, CypherValue>,
    /// Direction of the relationship.
    pub direction: RelDirection,
    /// Optional variable-length quantifier (e.g. `*1..5`).
    pub variable_length: Option<VariableLength>,
}

impl RelPattern {
    /// Construct an anonymous, untyped, property-less relationship with the
    /// given direction.
    pub(crate) fn simple(direction: RelDirection) -> Self {
        RelPattern {
            variable: None,
            rel_types: Vec::new(),
            properties: HashMap::new(),
            direction,
            variable_length: None,
        }
    }
}

/// A variable-length relationship quantifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableLength {
    /// Minimum hops (None = 0).
    pub min: Option<u64>,
    /// Maximum hops (None = unbounded).
    pub max: Option<u64>,
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

/// An expression that can appear in a `RETURN` clause, `WHERE` condition, or
/// other expression context.
#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    /// A bare variable reference (e.g. `n`).
    Variable(String),
    /// A single-level property access (e.g. `n.name`).
    Property(String, String),
    /// A nested property access (e.g. `n.address.city`).
    NestedProperty {
        variable: String,
        properties: Vec<String>,
    },
    /// The wildcard `*` (return everything).
    All,
    /// A literal value used as an expression (e.g. in arithmetic).
    Literal(CypherValue),
    /// Arithmetic addition: `left + right`.
    Add(Box<Expression>, Box<Expression>),
    /// Arithmetic subtraction: `left - right`.
    Sub(Box<Expression>, Box<Expression>),
    /// Arithmetic multiplication: `left * right`.
    Mul(Box<Expression>, Box<Expression>),
    /// Arithmetic division: `left / right`.
    Div(Box<Expression>, Box<Expression>),
    /// Arithmetic modulo: `left % right`.
    Mod(Box<Expression>, Box<Expression>),
    /// A list literal: `[1, 2, 3]`.
    List(Vec<Expression>),
    /// A map literal: `{name: "Alice", age: 30}`.
    Map(Vec<(String, Expression)>),
    /// A function call: `count(n)`, `collect(x)`, etc.
    FunctionCall { name: String, args: Vec<Expression> },
    /// A parameter reference: `$param`.
    Parameter(String),
    /// A CASE expression: `CASE WHEN x > 10 THEN 'big' ELSE 'small' END`.
    Case {
        arms: Vec<(Box<Expression>, Box<Expression>)>,
        default: Option<Box<Expression>>,
    },
    /// An aggregation: `count(*)`, `sum(n.age)`, etc.
    Aggregation {
        kind: AggregationKind,
        target: Box<Expression>,
        distinct: bool,
    },
    /// Comparison: `left > right`.
    Gt(Box<Expression>, Box<Expression>),
    /// Comparison: `left < right`.
    Lt(Box<Expression>, Box<Expression>),
    /// Comparison: `left >= right`.
    Gte(Box<Expression>, Box<Expression>),
    /// Comparison: `left <= right`.
    Lte(Box<Expression>, Box<Expression>),
    /// Comparison: `left <> right` or `left != right`.
    Neq(Box<Expression>, Box<Expression>),
}

/// The kind of aggregation function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Collect,
}

/// A basic `WHERE` expression (subset of OpenCypher).
#[derive(Debug, Clone, PartialEq)]
pub enum WhereExpr {
    /// An equality check: `expr = value`.
    Eq(Expression, CypherValue),
    /// A non-equality check: `expr <> value` or `expr != value`.
    Neq(Expression, CypherValue),
    /// A less-than check: `expr < value`.
    Lt(Expression, CypherValue),
    /// A greater-than check: `expr > value`.
    Gt(Expression, CypherValue),
    /// A less-than-or-equal check: `expr <= value`.
    Lte(Expression, CypherValue),
    /// A greater-than-or-equal check: `expr >= value`.
    Gte(Expression, CypherValue),
    /// Membership test: `expr IN [1, 2, 3]`.
    In(Expression, Expression),
    /// `IS NULL` check: `expr IS NULL`.
    IsNull(Expression),
    /// `IS NOT NULL` check: `expr IS NOT NULL`.
    IsNotNull(Expression),
    /// `STARTS WITH` string check: `expr STARTS WITH "prefix"`.
    StartsWith(Expression, Expression),
    /// `ENDS WITH` string check: `expr ENDS WITH "suffix"`.
    EndsWith(Expression, Expression),
    /// `CONTAINS` string check: `expr CONTAINS "sub"`.
    Contains(Expression, Expression),
    /// Boolean negation: `NOT expr`.
    Not(Box<WhereExpr>),
    /// Logical AND of two sub-expressions.
    And(Box<WhereExpr>, Box<WhereExpr>),
    /// Logical OR of two sub-expressions.
    Or(Box<WhereExpr>, Box<WhereExpr>),
    /// A bare expression evaluated as a boolean (truthy/falsy).
    Expr(Expression),
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
    Return {
        items: Vec<ReturnItem>,
        distinct: bool,
    },
    /// `[DETACH] DELETE variables`
    Delete {
        variables: Vec<String>,
        detach: bool,
    },
    /// `SET n.prop = value`
    SetProperty {
        variable: String,
        property: String,
        value: CypherValue,
    },
    /// `SET n += {map}` (merge properties)
    SetMerge {
        variable: String,
        properties: HashMap<String, CypherValue>,
    },
    /// `SET n:Label:Label2` (add labels)
    SetAddLabels {
        variable: String,
        labels: Vec<String>,
    },
    /// `REMOVE n.prop`
    RemoveProperty { variable: String, property: String },
    /// `REMOVE n:Label:Label2`
    RemoveLabels {
        variable: String,
        labels: Vec<String>,
    },
    /// `WITH items [WHERE ...] [ORDER BY ...] [SKIP n] [LIMIT n]`
    With {
        items: Vec<ReturnItem>,
        distinct: bool,
        where_clause: Option<WhereExpr>,
        order_by: Vec<OrderByItem>,
        skip: Option<u64>,
        limit: Option<u64>,
    },
}

/// A parsed OpenCypher query consisting of one or more clauses.
#[derive(Debug, Clone, PartialEq)]
pub struct CypherQuery {
    /// The ordered list of clauses in this query.
    pub clauses: Vec<Clause>,
    /// Optional `ORDER BY` clause items.
    pub order_by: Vec<OrderByItem>,
    /// Optional `SKIP` count.
    pub skip: Option<u64>,
    /// Optional `LIMIT` count.
    pub limit: Option<u64>,
}

/// A single item in an `ORDER BY` clause.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByItem {
    /// The expression to order by.
    pub expression: Expression,
    /// Sort direction (ascending or descending).
    pub direction: OrderDirection,
}

/// Sort direction for `ORDER BY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}
