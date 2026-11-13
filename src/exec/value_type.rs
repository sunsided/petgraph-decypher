//! `valueType()`: the GQL type of a value, normalized as Cypher does.

use std::cmp::Ordering;

use crate::value::Value;

/// A non-null type, in Cypher's type order: predefined types, then `MAP`,
/// `LIST<…>` (ordered by inner type), then `PATH`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Type {
    Boolean,
    String,
    Integer,
    Float,
    Node,
    Relationship,
    Map,
    List(Inner),
    Path,
}

/// The element type of a list: a normalized union of types whose members
/// are all nullable or all `NOT NULL`. No members is `NOTHING`, or `NULL`
/// when nullable.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inner {
    types: Vec<Type>,
    nullable: bool,
}

impl Inner {
    /// The element type of `items`.
    fn of(items: &[Value]) -> Self {
        let mut types: Vec<Type> = items.iter().filter_map(Type::of).collect();
        types.sort();
        types.dedup();
        // A member contained in another member is absorbed by it
        // (`LIST<INTEGER> | LIST<INTEGER | STRING>` is the latter).
        let absorbed: Vec<bool> = types
            .iter()
            .enumerate()
            .map(|(i, t)| types.iter().enumerate().any(|(j, u)| i != j && t.within(u)))
            .collect();
        let mut absorbed = absorbed.into_iter();
        types.retain(|_| !absorbed.next().unwrap_or(false));
        Self {
            types,
            nullable: items.iter().any(Value::is_null),
        }
    }

    /// Whether every value of `self` is a value of `other`.
    fn within(&self, other: &Inner) -> bool {
        (!self.nullable || other.nullable)
            && self
                .types
                .iter()
                .all(|t| other.types.iter().any(|u| t.within(u)))
    }

    fn render(&self, out: &mut String) {
        if self.types.is_empty() {
            out.push_str(if self.nullable { "NULL" } else { "NOTHING" });
            return;
        }
        for (i, t) in self.types.iter().enumerate() {
            if i > 0 {
                out.push_str(" | ");
            }
            t.render(out);
            if !self.nullable {
                out.push_str(" NOT NULL");
            }
        }
    }
}

/// `NOTHING` and `NULL` first, then single types (`NOT NULL` before
/// nullable), then unions: fewer members first, then by the first member
/// that differs.
impl Ord for Inner {
    fn cmp(&self, other: &Self) -> Ordering {
        let class = |i: &Inner| match i.types.len() {
            0 => u8::from(i.nullable),
            1 => 2,
            _ => 3,
        };
        class(self)
            .cmp(&class(other))
            .then_with(|| self.types.len().cmp(&other.types.len()))
            .then_with(|| self.types.cmp(&other.types))
            .then_with(|| self.nullable.cmp(&other.nullable))
    }
}

impl PartialOrd for Inner {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Type {
    /// The type of a value; `None` for `null`.
    fn of(value: &Value) -> Option<Self> {
        Some(match value {
            Value::Null => return None,
            Value::Boolean(_) => Type::Boolean,
            Value::String(_) => Type::String,
            Value::Integer(_) => Type::Integer,
            Value::Float(_) => Type::Float,
            Value::Node(_) => Type::Node,
            Value::Relationship(_) => Type::Relationship,
            Value::Map(_) => Type::Map,
            Value::List(items) => Type::List(Inner::of(items)),
            Value::Path(_) => Type::Path,
        })
    }

    fn within(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::List(a), Type::List(b)) => a.within(b),
            _ => self == other,
        }
    }

    fn render(&self, out: &mut String) {
        out.push_str(match self {
            Type::Boolean => "BOOLEAN",
            Type::String => "STRING",
            Type::Integer => "INTEGER",
            Type::Float => "FLOAT",
            Type::Node => "NODE",
            Type::Relationship => "RELATIONSHIP",
            Type::Map => "MAP",
            Type::Path => "PATH",
            Type::List(inner) => {
                out.push_str("LIST<");
                inner.render(out);
                out.push('>');
                return;
            }
        });
    }
}

/// The type `valueType(value)` reports, e.g. `INTEGER NOT NULL` or
/// `LIST<STRING NOT NULL | INTEGER NOT NULL> NOT NULL`.
pub(crate) fn value_type(value: &Value) -> String {
    let Some(t) = Type::of(value) else {
        return "NULL".into();
    };
    let mut out = String::new();
    t.render(&mut out);
    out.push_str(" NOT NULL");
    out
}
