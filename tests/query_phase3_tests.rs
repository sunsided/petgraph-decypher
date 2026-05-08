//! Integration tests for Phase 3: ORDER BY, SKIP, LIMIT, DISTINCT.

use petgraph_cypher::{
    build_graph_from_cypher, CypherValue, PetgraphCypher, QueryResult, ResultValue, Row,
};

fn collect_rows(result: QueryResult) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// DISTINCT execution
// ---------------------------------------------------------------------------

#[test]
fn query_return_distinct() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Alice"})
           CREATE (c:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN DISTINCT n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    // Two distinct names: Alice, Bob
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_return_not_distinct() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Alice"})
           CREATE (c:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    // Three rows (no dedup)
    assert_eq!(rows.len(), 3);
}

// ---------------------------------------------------------------------------
// ORDER BY execution
// ---------------------------------------------------------------------------

#[test]
fn query_order_by_ascending() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Charlie", age: 35})
           CREATE (b:Person {name: "Alice", age: 30})
           CREATE (c:Person {name: "Bob", age: 25})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name ORDER BY n.name ASC")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 3);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Alice");
    } else {
        panic!("expected string");
    }
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[2].values.get("name").unwrap() {
        assert_eq!(name, "Charlie");
    } else {
        panic!("expected string");
    }
}

#[test]
fn query_order_by_descending() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.age AS age ORDER BY n.age DESC")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 3);
    // Highest age first
    if let ResultValue::Scalar(CypherValue::Integer(age)) = rows[0].values.get("age").unwrap() {
        assert_eq!(*age, 35);
    } else {
        panic!("expected integer");
    }
}

// ---------------------------------------------------------------------------
// SKIP execution
// ---------------------------------------------------------------------------

#[test]
fn query_skip() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name SKIP 2")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_skip_beyond_end() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name SKIP 10")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// LIMIT execution
// ---------------------------------------------------------------------------

#[test]
fn query_limit() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name LIMIT 2")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_limit_zero() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name LIMIT 0")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// SKIP + LIMIT combined
// ---------------------------------------------------------------------------

#[test]
fn query_skip_and_limit() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})
           CREATE (d:Person {name: "Diana"})
           CREATE (e:Person {name: "Eve"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name SKIP 1 LIMIT 2")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// ORDER BY + SKIP + LIMIT combined
// ---------------------------------------------------------------------------

#[test]
fn query_order_skip_limit_combined() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Charlie", age: 35})
           CREATE (b:Person {name: "Alice", age: 30})
           CREATE (c:Person {name: "Bob", age: 25})
           CREATE (d:Person {name: "Diana", age: 40})
           CREATE (e:Person {name: "Eve", age: 20})"#,
    )
    .unwrap();

    // Order by age DESC, skip 1, limit 2: should give Diana(40), Charlie(35), Bob(25), Alice(30), Eve(20)
    // sorted DESC: Diana(40), Charlie(35), Alice(30), Bob(25), Eve(20)
    // skip 1: Charlie(35), Alice(30), Bob(25), Eve(20)
    // limit 2: Charlie(35), Alice(30)
    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name, n.age AS age ORDER BY n.age DESC SKIP 1 LIMIT 2")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
    if let ResultValue::Scalar(CypherValue::Integer(age)) = rows[0].values.get("age").unwrap() {
        assert_eq!(*age, 35); // Charlie
    } else {
        panic!("expected integer");
    }
    if let ResultValue::Scalar(CypherValue::Integer(age)) = rows[1].values.get("age").unwrap() {
        assert_eq!(*age, 30); // Alice
    } else {
        panic!("expected integer");
    }
}
