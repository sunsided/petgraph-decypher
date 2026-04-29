//! Integration tests for the graph builder.

use petgraph_cypher::{build_graph_from_cypher, CypherValue, NodeData};

#[test]
fn build_graph_single_node() {
    let g = build_graph_from_cypher("CREATE (n:Person {name: \"Alice\"})").unwrap();
    assert_eq!(g.node_count(), 1);
    assert_eq!(g.edge_count(), 0);
    let n = g.node_indices().next().unwrap();
    let data: &NodeData = &g[n];
    assert_eq!(data.labels, vec!["Person"]);
    assert_eq!(
        data.properties.get("name"),
        Some(&CypherValue::String("Alice".into()))
    );
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
    assert_eq!(g[e].rel_type.as_deref(), Some("KNOWS"));
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
    // A plain MATCH with no CREATE should produce an empty graph.
    let g = build_graph_from_cypher("MATCH (n:Person)-[:KNOWS]->(m)").unwrap();
    assert_eq!(g.node_count(), 0);
    assert_eq!(g.edge_count(), 0);
}

#[test]
fn build_graph_left_directed_edge_direction() {
    let g = build_graph_from_cypher("CREATE (a)<-[:LIKES]-(b)").unwrap();
    assert_eq!(g.edge_count(), 1);
    let e = g.edge_indices().next().unwrap();
    // Left direction: b → a in petgraph terms
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
    assert_eq!(
        g[e].properties.get("since"),
        Some(&CypherValue::Integer(2020))
    );
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
    assert_eq!(data.properties.get("age"), Some(&CypherValue::Integer(30)));
}
