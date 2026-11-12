//! Cypher values and their comparison semantics.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use petgraph::graph::{EdgeIndex, NodeIndex};

/// A property map: property names to values.
pub type Properties = BTreeMap<String, Value>;

/// A Cypher value: a property value, a query parameter, or a cell of a query
/// result.
///
/// Graph elements are referenced by their petgraph index, so a result row
/// keeps node and relationship identity; look their data up in the graph
/// (`graph[node]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// `null`.
    #[default]
    Null,
    /// `true` / `false`.
    Boolean(bool),
    /// A 64-bit signed integer.
    Integer(i64),
    /// A 64-bit float.
    Float(f64),
    /// A UTF-8 string.
    String(String),
    /// A list of values.
    List(Vec<Value>),
    /// A map with string keys.
    Map(BTreeMap<String, Value>),
    /// A node of the queried graph.
    Node(NodeIndex),
    /// A relationship (edge) of the queried graph.
    Relationship(EdgeIndex),
    /// A path through the queried graph.
    Path(Path),
}

/// A path: alternating nodes and relationships, starting and ending with a
/// node. `nodes.len() == relationships.len() + 1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path {
    /// The nodes along the path, in traversal order.
    pub nodes: Vec<NodeIndex>,
    /// The relationships along the path, in traversal order.
    pub relationships: Vec<EdgeIndex>,
}

impl Value {
    /// Whether the value is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// The Cypher type name, as used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Boolean(_) => "BOOLEAN",
            Value::Integer(_) => "INTEGER",
            Value::Float(_) => "FLOAT",
            Value::String(_) => "STRING",
            Value::List(_) => "LIST",
            Value::Map(_) => "MAP",
            Value::Node(_) => "NODE",
            Value::Relationship(_) => "RELATIONSHIP",
            Value::Path(_) => "PATH",
        }
    }

    /// Whether the value may be stored as a property: `null`, a scalar, or a
    /// list of scalars of one type.
    pub fn is_property_value(&self) -> bool {
        match self {
            Value::Null
            | Value::Boolean(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::String(_) => true,
            Value::List(items) => items.iter().all(|v| {
                matches!(
                    v,
                    Value::Boolean(_) | Value::Integer(_) | Value::Float(_) | Value::String(_)
                )
            }),
            _ => false,
        }
    }

    /// Cypher equality (`=`): `None` when the result is `null`.
    ///
    /// Integers and floats compare numerically, `NaN` equals nothing, and
    /// lists and maps compare element-wise with `null` propagation.
    pub fn cypher_eq(&self, other: &Value) -> Option<bool> {
        match (self, other) {
            (Value::Null, _) | (_, Value::Null) => None,
            (Value::Integer(a), Value::Integer(b)) => Some(a == b),
            (Value::Integer(_) | Value::Float(_), Value::Integer(_) | Value::Float(_)) => {
                Some(self.as_f64() == other.as_f64())
            }
            (Value::List(a), Value::List(b)) => {
                if a.len() != b.len() {
                    return Some(false);
                }
                all_eq(a.iter().zip(b))
            }
            (Value::Map(a), Value::Map(b)) => {
                if a.len() != b.len() || a.keys().ne(b.keys()) {
                    return Some(false);
                }
                all_eq(a.values().zip(b.values()))
            }
            _ => Some(self == other),
        }
    }

    /// Cypher comparison (`<`, `>`, …): `None` when the values are not
    /// comparable (different types, `null`, or `NaN`).
    pub fn cypher_cmp(&self, other: &Value) -> Option<Ordering> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Some(a.cmp(b)),
            (Value::Integer(_) | Value::Float(_), Value::Integer(_) | Value::Float(_)) => {
                self.as_f64()?.partial_cmp(&other.as_f64()?)
            }
            (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
            (Value::Boolean(a), Value::Boolean(b)) => Some(a.cmp(b)),
            (Value::List(a), Value::List(b)) => {
                for (x, y) in a.iter().zip(b) {
                    match x.cypher_cmp(y)? {
                        Ordering::Equal => {}
                        other => return Some(other),
                    }
                }
                Some(a.len().cmp(&b.len()))
            }
            _ => None,
        }
    }

    /// The total order `ORDER BY`, `min` and `max` use: maps, nodes,
    /// relationships, lists, paths, strings, booleans, numbers (`NaN` last),
    /// then `null`.
    pub fn order(&self, other: &Value) -> Ordering {
        let rank = |v: &Value| match v {
            Value::Map(_) => 0,
            Value::Node(_) => 1,
            Value::Relationship(_) => 2,
            Value::List(_) => 3,
            Value::Path(_) => 4,
            Value::String(_) => 5,
            Value::Boolean(_) => 6,
            Value::Integer(_) | Value::Float(_) => 7,
            Value::Null => 8,
        };
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Integer(_) | Value::Float(_), Value::Integer(_) | Value::Float(_)) => {
                let (a, b) = (self.as_f64().unwrap(), other.as_f64().unwrap());
                match (a.is_nan(), b.is_nan()) {
                    (true, true) => Ordering::Equal,
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
                }
            }
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Boolean(a), Value::Boolean(b)) => a.cmp(b),
            (Value::List(a), Value::List(b)) => a
                .iter()
                .zip(b)
                .map(|(x, y)| x.order(y))
                .find(|o| o.is_ne())
                .unwrap_or_else(|| a.len().cmp(&b.len())),
            (Value::Map(a), Value::Map(b)) => a
                .iter()
                .zip(b)
                .map(|((ka, va), (kb, vb))| ka.cmp(kb).then_with(|| va.order(vb)))
                .find(|o| o.is_ne())
                .unwrap_or_else(|| a.len().cmp(&b.len())),
            (Value::Node(a), Value::Node(b)) => a.cmp(b),
            (Value::Relationship(a), Value::Relationship(b)) => a.cmp(b),
            (Value::Path(a), Value::Path(b)) => a
                .nodes
                .cmp(&b.nodes)
                .then_with(|| a.relationships.cmp(&b.relationships)),
            _ => rank(self).cmp(&rank(other)),
        }
    }

    /// Equivalence used for grouping, `DISTINCT` and `UNION`: like `=`, but
    /// `null` equals `null` and `NaN` equals `NaN`.
    pub fn equivalent(&self, other: &Value) -> bool {
        self.order(other) == Ordering::Equal
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
}

fn all_eq<'a>(pairs: impl Iterator<Item = (&'a Value, &'a Value)>) -> Option<bool> {
    let mut saw_null = false;
    for (a, b) in pairs {
        match a.cypher_eq(b) {
            Some(false) => return Some(false),
            None => saw_null = true,
            Some(true) => {}
        }
    }
    if saw_null { None } else { Some(true) }
}

impl fmt::Display for Value {
    /// Renders the value as a Cypher literal; graph elements render as
    /// `Node(i)`, `Relationship(i)` and `Path(...)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Boolean(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Float(x) if x.is_nan() => write!(f, "NaN"),
            Value::Float(x) if x.is_infinite() => {
                write!(f, "{}", if *x > 0.0 { "Infinity" } else { "-Infinity" })
            }
            Value::Float(x) => write!(f, "{x:?}"),
            Value::String(s) => write!(f, "'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
            Value::Node(n) => write!(f, "Node({})", n.index()),
            Value::Relationship(r) => write!(f, "Relationship({})", r.index()),
            Value::Path(p) => write!(f, "Path({:?}, {:?})", p.nodes, p.relationships),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Boolean(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(v.into())
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl From<NodeIndex> for Value {
    fn from(v: NodeIndex) -> Self {
        Value::Node(v)
    }
}

impl From<EdgeIndex> for Value {
    fn from(v: EdgeIndex) -> Self {
        Value::Relationship(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_follows_cypher() {
        assert_eq!(Value::from(1).cypher_eq(&Value::from(1.0)), Some(true));
        assert_eq!(Value::Null.cypher_eq(&Value::Null), None);
        assert_eq!(
            Value::from(f64::NAN).cypher_eq(&Value::from(f64::NAN)),
            Some(false)
        );
        let with_null = Value::List(vec![Value::from(1), Value::Null]);
        assert_eq!(with_null.cypher_eq(&Value::from(vec![1, 2])), None);
        assert_eq!(with_null.cypher_eq(&Value::from(vec![2, 2])), Some(false));
        assert_eq!(Value::from("a").cypher_eq(&Value::from(1)), Some(false));
    }

    #[test]
    fn ordering_is_total() {
        let mut values = [
            Value::Null,
            Value::from(f64::NAN),
            Value::from(2),
            Value::from(1.5),
            Value::from(true),
            Value::from("a"),
            Value::from(vec![1]),
        ];
        values.sort_by(Value::order);
        assert_eq!(
            values.iter().map(Value::type_name).collect::<Vec<_>>(),
            [
                "LIST", "STRING", "BOOLEAN", "FLOAT", "INTEGER", "FLOAT", "NULL"
            ]
        );
        assert!(Value::Null.equivalent(&Value::Null));
        assert!(Value::from(1).equivalent(&Value::from(1.0)));
        assert!(!Value::from(1).equivalent(&Value::from(2)));
    }
}
