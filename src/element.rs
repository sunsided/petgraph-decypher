//! Node and relationship weights: what the engine needs to read them, and to
//! create and modify them in mutating queries.

use crate::value::{Properties, Value};

/// Read access to a node weight. Implement it for your own node type to query
/// a `Graph<N, E>` with custom weights.
pub trait CypherNode {
    /// The node's labels.
    fn labels(&self) -> impl Iterator<Item = &str>;

    /// Whether the node has `label`.
    fn has_label(&self, label: &str) -> bool {
        self.labels().any(|l| l == label)
    }

    /// The value of property `key`, if set.
    fn property(&self, key: &str) -> Option<&Value>;

    /// All properties.
    fn properties(&self) -> impl Iterator<Item = (&str, &Value)>;
}

/// Read access to a relationship (edge) weight.
pub trait CypherEdge {
    /// The relationship type, e.g. `KNOWS`.
    fn rel_type(&self) -> &str;

    /// The value of property `key`, if set.
    fn property(&self, key: &str) -> Option<&Value>;

    /// All properties.
    fn properties(&self) -> impl Iterator<Item = (&str, &Value)>;
}

/// Node weights that mutating queries (`CREATE`, `SET`, …) can build and
/// modify.
pub trait CypherNodeMut: CypherNode {
    /// Builds a node from a `CREATE`/`MERGE` pattern. `variable` is the
    /// pattern variable (`n` in `(n:Person)`), if any.
    fn from_cypher(variable: Option<&str>, labels: Vec<String>, properties: Properties) -> Self;

    /// Adds `label` unless the node already has it.
    fn add_label(&mut self, label: &str);

    /// Removes `label` if present.
    fn remove_label(&mut self, label: &str);

    /// Sets property `key`; `Value::Null` removes it.
    fn set_property(&mut self, key: &str, value: Value);

    /// Removes every property.
    fn clear_properties(&mut self);
}

/// Relationship weights that mutating queries can build and modify.
pub trait CypherEdgeMut: CypherEdge {
    /// Builds a relationship from a `CREATE`/`MERGE` pattern. `variable` is the
    /// pattern variable (`r` in `-[r:KNOWS]->`), if any.
    fn from_cypher(variable: Option<&str>, rel_type: String, properties: Properties) -> Self;

    /// Sets property `key`; `Value::Null` removes it.
    fn set_property(&mut self, key: &str, value: Value);

    /// Removes every property.
    fn clear_properties(&mut self);
}

/// The default node weight.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeData {
    /// The pattern variable the node was created with (e.g. `"n"`), if any.
    pub variable: Option<String>,
    /// Node labels (e.g. `["Person", "Employee"]`).
    pub labels: Vec<String>,
    /// Node properties.
    pub properties: Properties,
}

/// The default relationship weight.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EdgeData {
    /// The pattern variable the relationship was created with, if any.
    pub variable: Option<String>,
    /// Relationship type (e.g. `"KNOWS"`).
    pub rel_type: String,
    /// Relationship properties.
    pub properties: Properties,
}

impl CypherNode for NodeData {
    fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().map(String::as_str)
    }

    fn property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    fn properties(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl CypherNodeMut for NodeData {
    fn from_cypher(variable: Option<&str>, labels: Vec<String>, properties: Properties) -> Self {
        Self {
            variable: variable.map(str::to_owned),
            labels,
            properties,
        }
    }

    fn add_label(&mut self, label: &str) {
        if !self.has_label(label) {
            self.labels.push(label.to_owned());
        }
    }

    fn remove_label(&mut self, label: &str) {
        self.labels.retain(|l| l != label);
    }

    fn set_property(&mut self, key: &str, value: Value) {
        set(&mut self.properties, key, value);
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
}

impl CypherEdge for EdgeData {
    fn rel_type(&self) -> &str {
        &self.rel_type
    }

    fn property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    fn properties(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl CypherEdgeMut for EdgeData {
    fn from_cypher(variable: Option<&str>, rel_type: String, properties: Properties) -> Self {
        Self {
            variable: variable.map(str::to_owned),
            rel_type,
            properties,
        }
    }

    fn set_property(&mut self, key: &str, value: Value) {
        set(&mut self.properties, key, value);
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
}

fn set(properties: &mut Properties, key: &str, value: Value) {
    if value.is_null() {
        properties.remove(key);
    } else {
        properties.insert(key.to_owned(), value);
    }
}
