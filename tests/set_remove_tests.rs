//! Integration tests for SET/REMOVE clauses.

use petgraph_cypher::{
    CypherValue, PetgraphCypher, QueryResult, ResultValue, Row, build_graph_from_cypher,
};

fn collect_rows(result: QueryResult) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// SET property execution
// ---------------------------------------------------------------------------

#[test]
fn query_set_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", age: 30})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) WHERE n.name = "Alice" SET n.age = 31"#)
        .unwrap();

    let result = g.cypher("MATCH (n:Person) RETURN n.age AS age").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(age)) = rows[0].values.get("age").unwrap() {
        assert_eq!(*age, 31);
    } else {
        panic!("expected integer age");
    }
}

#[test]
fn query_set_property_on_multiple_nodes() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    g.cypher_mut("MATCH (n:Person) SET n.active = true")
        .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.active AS active")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        if let ResultValue::Scalar(CypherValue::Boolean(active)) = row.values.get("active").unwrap()
        {
            assert!(*active);
        } else {
            panic!("expected boolean");
        }
    }
}

// ---------------------------------------------------------------------------
// SET merge properties
// ---------------------------------------------------------------------------

#[test]
fn query_set_merge_properties() {
    let mut g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", age: 30})"#).unwrap();

    g.cypher_mut(
        r#"MATCH (n:Person) WHERE n.name = "Alice" SET n += {city: "Berlin", role: "Dev"}"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.city AS city, n.role AS role, n.age AS age")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(city)) = rows[0].values.get("city").unwrap() {
        assert_eq!(city, "Berlin");
    } else {
        panic!("expected string");
    }
}

// ---------------------------------------------------------------------------
// SET add labels
// ---------------------------------------------------------------------------

#[test]
fn query_set_add_labels() {
    let mut g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) WHERE n.name = "Alice" SET n:Employee:Manager"#)
        .unwrap();

    let result = g
        .cypher("MATCH (n:Employee) RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);

    let result = g.cypher("MATCH (n:Manager) RETURN n.name AS name").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

// ---------------------------------------------------------------------------
// REMOVE property
// ---------------------------------------------------------------------------

#[test]
fn query_remove_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", age: 30})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) WHERE n.name = "Alice" REMOVE n.age"#)
        .unwrap();

    let result = g.cypher("MATCH (n:Person) RETURN n.age AS age").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    // After removal, age should be null
    if let ResultValue::Scalar(CypherValue::Null) = rows[0].values.get("age").unwrap() {
        // Good - property was removed
    } else {
        panic!("expected null after removal");
    }
}

// ---------------------------------------------------------------------------
// REMOVE labels
// ---------------------------------------------------------------------------

#[test]
fn query_remove_labels() {
    let mut g =
        build_graph_from_cypher(r#"CREATE (a:Person:Employee:Manager {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Manager) WHERE n.name = "Alice" REMOVE n:Manager"#)
        .unwrap();

    // Should still match as Employee
    let result = g
        .cypher("MATCH (n:Employee) RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);

    // Should NOT match as Manager anymore
    let result = g.cypher("MATCH (n:Manager) RETURN n.name AS name").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 0);
}

// ---------------------------------------------------------------------------
// Error: SET in read query
// ---------------------------------------------------------------------------

#[test]
fn query_read_rejects_set() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    let result = g.cypher("MATCH (n) SET n.age = 30 RETURN n");
    assert!(result.is_err());
}

#[test]
fn query_read_rejects_remove() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    let result = g.cypher("MATCH (n) REMOVE n.name RETURN n");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// SET on edge properties
// ---------------------------------------------------------------------------

#[test]
fn query_set_edge_property() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    g.cypher_mut(r#"MATCH (a)-[r:KNOWS]->(b) WHERE a.name = "Alice" SET r.since = 2021"#)
        .unwrap();

    let result = g
        .cypher("MATCH (a)-[r:KNOWS]->(b) RETURN r.since AS since")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(since)) = rows[0].values.get("since").unwrap() {
        assert_eq!(*since, 2021);
    } else {
        panic!("expected integer");
    }
}

#[test]
fn query_remove_edge_property() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    g.cypher_mut(r#"MATCH (a)-[r:KNOWS]->(b) REMOVE r.since"#)
        .unwrap();

    let result = g
        .cypher("MATCH (a)-[r:KNOWS]->(b) RETURN r.since AS since")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Null) = rows[0].values.get("since").unwrap() {
        // Good - property was removed
    } else {
        panic!("expected null after removal");
    }
}
