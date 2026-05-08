//! Integration tests for Phase 1+ query execution features.

use petgraph_cypher::{
    CypherValue, PetgraphCypher, QueryResult, ResultValue, Row, build_graph_from_cypher,
};

fn collect_rows(result: QueryResult) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Rich WHERE execution
// ---------------------------------------------------------------------------

#[test]
fn query_where_neq_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name <> "Alice" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_lt_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) WHERE n.age < 30 RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Bob");
    } else {
        panic!("expected string");
    }
}

#[test]
fn query_where_gte_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) WHERE n.age >= 30 RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_is_null_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: null})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) WHERE n.name IS NULL RETURN n")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_is_not_null_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: null})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) WHERE n.name IS NOT NULL RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_starts_with_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Alex"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name STARTS WITH "Al" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_ends_with_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Charlie"})
           CREATE (c:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name ENDS WITH "e" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_contains_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name CONTAINS "li" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_not_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE NOT n.name = "Alice" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_or_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(
            r#"MATCH (n:Person) WHERE n.name = "Alice" OR n.name = "Bob" RETURN n.name AS name"#,
        )
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_and_or_precedence_execution() {
    // (name = "A" AND age = 30) OR (name = "B" AND age = 25)
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 30})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name = "Alice" AND n.age = 30 OR n.name = "Bob" AND n.age = 25 RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_parenthesized_execution() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n:Person) WHERE (n.age < 30 OR n.age > 34) RETURN n.name AS name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// Arithmetic execution
// ---------------------------------------------------------------------------

#[test]
fn query_arithmetic_add() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", age: 30})"#).unwrap();

    let result = g
        .cypher("MATCH (n:Person) RETURN n.age + 5 AS adjusted")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(val)) = rows[0].values.get("adjusted").unwrap()
    {
        assert_eq!(*val, 35);
    } else {
        panic!("expected integer");
    }
}

#[test]
fn query_arithmetic_mul() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", score: 7})"#).unwrap();

    let result = g.cypher("MATCH (n) RETURN n.score * 3 AS tripled").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(val)) = rows[0].values.get("tripled").unwrap() {
        assert_eq!(*val, 21);
    } else {
        panic!("expected integer");
    }
}

#[test]
fn query_arithmetic_div() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice", score: 21})"#).unwrap();

    let result = g.cypher("MATCH (n) RETURN n.score / 3 AS third").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(val)) = rows[0].values.get("third").unwrap() {
        assert_eq!(*val, 7);
    } else {
        panic!("expected integer");
    }
}

#[test]
fn query_arithmetic_precedence() {
    // 10 + 5 * 2 = 20 (not 30)
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", base: 10, bonus: 5, multiplier: 2})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (n) RETURN n.base + n.bonus * n.multiplier AS result")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(val)) = rows[0].values.get("result").unwrap() {
        assert_eq!(*val, 20);
    } else {
        panic!("expected integer");
    }
}
