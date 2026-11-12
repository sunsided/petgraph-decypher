//! Integration tests for the graph builder.

use petgraph_decypher::{
    CypherEdge, CypherEdgeMut, CypherError, CypherNode, CypherNodeMut, NodeData, Properties, Value,
    build_graph_from_cypher, build_graph_from_cypher_typed,
};

#[test]
fn build_graph_single_node() {
    let g = build_graph_from_cypher("CREATE (n:Person {name: \"Alice\"})").unwrap();
    assert_eq!(g.node_count(), 1);
    assert_eq!(g.edge_count(), 0);
    let n = g.node_indices().next().unwrap();
    let data: &NodeData = &g[n];
    assert_eq!(data.labels, vec!["Person"]);
    assert_eq!(data.properties.get("name"), Some(&Value::from("Alice")));
}

#[test]
fn build_graph_two_nodes_one_edge() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
    let e = g.edge_indices().next().unwrap();
    assert_eq!(g[e].rel_type, "KNOWS");
}

#[test]
fn build_graph_shared_variable_reuses_node() {
    // 'a' appears in both patterns but should only produce one node.
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})
           CREATE (a)-[:LIKES]->(c:Thing {name: "Coffee"})"#,
    )
    .unwrap();
    assert_eq!(g.node_count(), 3);
    assert_eq!(g.edge_count(), 2);
}

#[test]
fn build_graph_chain_of_nodes() {
    let g = build_graph_from_cypher("CREATE (a:X)-[:E]->(b:Y)-[:E]->(c:Z)").unwrap();
    assert_eq!(g.node_count(), 3);
    assert_eq!(g.edge_count(), 2);
}

#[test]
fn build_graph_merge_creates_nodes() {
    let g = build_graph_from_cypher("MERGE (n:Person {name: \"Charlie\"})").unwrap();
    assert_eq!(g.node_count(), 1);
}

#[test]
fn build_graph_match_does_not_add_nodes() {
    // A MATCH with no CREATE should produce an empty graph.
    let g = build_graph_from_cypher("MATCH (n:Person)-[:KNOWS]->(m) RETURN m").unwrap();
    assert_eq!(g.node_count(), 0);
    assert_eq!(g.edge_count(), 0);
}

#[test]
fn build_graph_rejects_query_without_return_or_update() {
    // openCypher: a query must end with RETURN or an updating clause.
    let result = build_graph_from_cypher("MATCH (n:Person)-[:KNOWS]->(m)");
    assert!(
        matches!(result, Err(CypherError::ParseError(_))),
        "{result:?}"
    );
}

#[test]
fn build_graph_left_directed_edge_direction() {
    let g = build_graph_from_cypher("CREATE (a)<-[:LIKES]-(b)").unwrap();
    assert_eq!(g.edge_count(), 1);
    let e = g.edge_indices().next().unwrap();
    // `(a)<-[:LIKES]-(b)` means the edge runs b → a, so b is the source.
    let (src, tgt) = g.edge_endpoints(e).unwrap();
    let src_data = &g[src];
    let tgt_data = &g[tgt];
    assert_eq!(src_data.variable.as_deref(), Some("b"));
    assert_eq!(tgt_data.variable.as_deref(), Some("a"));
}

#[test]
fn build_graph_edge_with_properties() {
    let g = build_graph_from_cypher(r#"CREATE (a)-[:KNOWS {since: 2020}]->(b)"#).unwrap();
    let e = g.edge_indices().next().unwrap();
    assert_eq!(g[e].properties.get("since"), Some(&Value::Integer(2020)));
}

#[test]
fn build_graph_anonymous_node() {
    // Anonymous nodes (no variable) always create new nodes.
    let g = build_graph_from_cypher("CREATE ()-[:X]->()").unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn build_graph_node_variable_data() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person:Employee {age: 30})"#).unwrap();
    let n = g.node_indices().next().unwrap();
    let data = &g[n];
    assert_eq!(data.variable.as_deref(), Some("n"));
    assert!(data.labels.contains(&"Person".to_string()));
    assert!(data.labels.contains(&"Employee".to_string()));
    assert_eq!(data.properties.get("age"), Some(&Value::Integer(30)));
}

/// Minimal custom node type for testing `build_graph_from_cypher_typed`.
#[derive(Debug, Clone, PartialEq)]
struct CustomNode {
    variable: Option<String>,
    labels: Vec<String>,
    properties: Properties,
}

impl CypherNode for CustomNode {
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

impl CypherNodeMut for CustomNode {
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
        if value.is_null() {
            self.properties.remove(key);
        } else {
            self.properties.insert(key.to_owned(), value);
        }
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
}

/// Minimal custom edge type for testing `build_graph_from_cypher_typed`.
#[derive(Debug, Clone, PartialEq)]
struct CustomEdge {
    variable: Option<String>,
    rel_type: String,
    properties: Properties,
}

impl CypherEdge for CustomEdge {
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

impl CypherEdgeMut for CustomEdge {
    fn from_cypher(variable: Option<&str>, rel_type: String, properties: Properties) -> Self {
        Self {
            variable: variable.map(str::to_owned),
            rel_type,
            properties,
        }
    }

    fn set_property(&mut self, key: &str, value: Value) {
        if value.is_null() {
            self.properties.remove(key);
        } else {
            self.properties.insert(key.to_owned(), value);
        }
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
}

#[test]
fn build_graph_typed_with_custom_types() {
    let g = build_graph_from_cypher_typed::<CustomNode, CustomEdge>(
        r#"CREATE (a:Person {name: "Alice"})-[r:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);

    let nodes: Vec<_> = g.node_weights().collect();
    assert!(nodes.iter().all(|n| n.has_label("Person")));
    assert!(
        nodes
            .iter()
            .any(|n| n.property("name") == Some(&Value::from("Alice")))
    );
    assert!(
        nodes
            .iter()
            .any(|n| n.property("name") == Some(&Value::from("Bob")))
    );

    let edge = g.edge_indices().next().unwrap();
    assert_eq!(g[edge].rel_type(), "KNOWS");
    assert_eq!(g[edge].property("since"), Some(&Value::Integer(2020)));
    assert_eq!(g[edge].variable.as_deref(), Some("r"));
    let (src, _) = g.edge_endpoints(edge).unwrap();
    assert_eq!(g[src].variable.as_deref(), Some("a"));
}

#[test]
fn build_graph_propagates_errors() {
    let result = build_graph_from_cypher("CREATE (a)-[:X]-(b)");
    assert!(
        matches!(result, Err(CypherError::InvalidQuery(_))),
        "{result:?}"
    );
}
