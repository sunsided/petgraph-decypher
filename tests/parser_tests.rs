//! Integration tests for preparing queries: parsing and semantic analysis via
//! [`Query::new`].

use petgraph::Graph;
use petgraph_decypher::{CypherError, EdgeData, NodeData, Parameters, Query, Value};

fn prepare(text: &str) -> Query {
    Query::new(text).unwrap_or_else(|e| panic!("{text:?} should prepare: {e}"))
}

/// Runs a mutating query on an empty graph and returns the graph.
fn run_mut(text: &str) -> Graph<NodeData, EdgeData> {
    let mut graph = Graph::new();
    prepare(text)
        .execute_mut(&mut graph, &Parameters::new())
        .unwrap();
    graph
}

#[test]
fn parse_create_single_node() {
    let q = prepare("CREATE (n:Person {name: \"Alice\"})");
    assert!(!q.is_read_only());
    assert_eq!(q.parameters().count(), 0);
}

#[test]
fn parse_create_two_nodes_relationship() {
    let q = prepare(r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#);
    assert!(!q.is_read_only());
}

#[test]
fn parse_create_left_directed_relationship() {
    let g = run_mut("CREATE (a)<-[:LIKES]-(b)");
    let e = g.edge_indices().next().unwrap();
    let (src, tgt) = g.edge_endpoints(e).unwrap();
    assert_eq!(g[e].rel_type, "LIKES");
    assert_eq!(g[src].variable.as_deref(), Some("b"));
    assert_eq!(g[tgt].variable.as_deref(), Some("a"));
}

#[test]
fn parse_create_undirected_relationship_is_rejected() {
    // openCypher: CREATE needs a direction for every relationship.
    let result = Query::new("CREATE (a)-[:PEER]-(b)");
    assert!(
        matches!(result, Err(CypherError::InvalidQuery(_))),
        "{result:?}"
    );
}

#[test]
fn parse_create_untyped_relationship_is_rejected() {
    // openCypher: CREATE needs exactly one type for every relationship.
    for text in ["CREATE (a)-->(b)", "CREATE (a)-[:A|B]->(b)"] {
        let result = Query::new(text);
        assert!(
            matches!(result, Err(CypherError::InvalidQuery(_))),
            "{text}: {result:?}"
        );
    }
}

#[test]
fn parse_create_multiple_labels() {
    let g = run_mut("CREATE (n:Person:Employee)");
    let n = g.node_indices().next().unwrap();
    assert_eq!(g[n].labels, ["Person", "Employee"]);
}

#[test]
fn parse_create_integer_and_float_properties() {
    let g = run_mut("CREATE (n:Item {count: 42, price: 9.99})");
    let props = &g[g.node_indices().next().unwrap()].properties;
    assert_eq!(props.get("count"), Some(&Value::Integer(42)));
    assert_eq!(props.get("price"), Some(&Value::Float(9.99)));
}

#[test]
fn parse_create_boolean_and_null_properties() {
    let g = run_mut("CREATE (n {active: true, deleted: false, memo: null})");
    let props = &g[g.node_indices().next().unwrap()].properties;
    assert_eq!(props.get("active"), Some(&Value::Boolean(true)));
    assert_eq!(props.get("deleted"), Some(&Value::Boolean(false)));
    // openCypher: a null property value means "no property".
    assert_eq!(props.get("memo"), None);
}

#[test]
fn parse_match_clause() {
    let q = prepare("MATCH (n:Person)-[r:KNOWS]->(m) RETURN m");
    assert!(q.is_read_only());
}

#[test]
fn parse_match_without_return_is_rejected() {
    // openCypher: a reading query must end with RETURN.
    let result = Query::new("MATCH (n:Person)-[r:KNOWS]->(m)");
    assert!(
        matches!(result, Err(CypherError::ParseError(_))),
        "{result:?}"
    );
}

#[test]
fn parse_match_with_where() {
    let q = prepare("MATCH (n:Person) WHERE n.name = \"Alice\" RETURN n");
    assert!(q.is_read_only());
    assert_eq!(q.parameters().count(), 0);
}

#[test]
fn parse_parameter_names() {
    let q = prepare("MATCH (n:Person) WHERE n.name = $name AND n.age >= $min_age RETURN n");
    let mut names: Vec<_> = q.parameters().collect();
    names.sort_unstable();
    assert_eq!(names, ["min_age", "name"]);
}

#[test]
fn parse_unresolved_variable_is_rejected() {
    let result = Query::new("MATCH (n) RETURN m");
    assert!(
        matches!(result, Err(CypherError::InvalidQuery(_))),
        "{result:?}"
    );
}

#[test]
fn parse_merge_clause() {
    let q = prepare("MERGE (n:Person {name: \"Alice\"})");
    assert!(!q.is_read_only());
}

#[test]
fn parse_return_clause() {
    let q = prepare("MATCH (n) RETURN n, n.name AS name, n.age");
    let result = q
        .execute(&Graph::<NodeData, EdgeData>::new(), &Parameters::new())
        .unwrap();
    assert_eq!(result.columns(), ["n", "name", "n.age"]);
}

#[test]
fn parse_delete_clause() {
    let q = prepare("MATCH (n) DELETE n");
    assert!(!q.is_read_only());
}

#[test]
fn parse_detach_delete_clause() {
    let q = prepare("MATCH (n) DETACH DELETE n");
    assert!(!q.is_read_only());
}

#[test]
fn parse_multiple_clauses() {
    let q = prepare(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           RETURN a, b"#,
    );
    assert!(!q.is_read_only());
}

#[test]
fn parse_semicolon_separated_statements() {
    let g = run_mut("CREATE (a:Person {name: \"Alice\"}); CREATE (b:Person {name: \"Bob\"})");
    assert_eq!(g.node_count(), 2);
}

#[test]
fn parse_error_on_invalid_input() {
    let result = Query::new("THIS IS NOT VALID CYPHER !!!");
    assert!(
        matches!(result, Err(CypherError::ParseError(_))),
        "{result:?}"
    );
}
