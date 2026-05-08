//! Integration tests for Phase 4: Variable-length relationships.

use petgraph_cypher::{
    build_graph_from_cypher, CypherValue, PetgraphCypher, QueryResult, ResultValue, Row,
};

fn collect_rows(result: QueryResult) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Variable-length relationship parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_variable_length_bare_star() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r*]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.variable_length.is_some());
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(0));
    assert!(vl.max.is_none());
}

#[test]
fn parse_variable_length_exact() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r*3]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(3));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_variable_length_range() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r*1..5]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(1));
    assert_eq!(vl.max, Some(5));
}

#[test]
fn parse_variable_length_min_only() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r*2..]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(2));
    assert!(vl.max.is_none());
}

#[test]
fn parse_variable_length_max_only() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r*..3]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(0));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_variable_length_with_type() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:KNOWS*1..3]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types.first().map(|s| s.as_str()), Some("KNOWS"));
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(1));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_variable_length_no_rel_var() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[*1..2]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.variable.is_none());
    assert!(rel.variable_length.is_some());
}

// ---------------------------------------------------------------------------
// Variable-length relationship execution
// ---------------------------------------------------------------------------

#[test]
fn query_var_length_two_hops() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})-[:KNOWS]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    // Match 1..2 hops: Alice->Bob (1 hop), Alice->Bob->Charlie (2 hops)
    let result = g
        .cypher("MATCH (a:Person)-[*1..2]->(b:Person) RETURN a.name AS from, b.name AS to")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    // Should find: (Alice, Bob) at 1 hop, (Alice, Charlie) at 2 hops
    // Also: (Bob, Charlie) at 1 hop
    assert!(rows.len() >= 2);
}

#[test]
fn query_var_length_exact_two() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})-[:KNOWS]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    // Exact 2 hops: only Alice->Bob->Charlie
    let result = g
        .cypher("MATCH (a)-[*2]->(b) RETURN a.name AS from, b.name AS to")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.len() >= 1);
    // At minimum, Alice->Charlie should be found
    let found_alice_charlie = rows.iter().any(|r| {
        let from = r.values.get("from").and_then(|v| match v {
            ResultValue::Scalar(CypherValue::String(s)) => Some(s.as_str()),
            _ => None,
        });
        let to = r.values.get("to").and_then(|v| match v {
            ResultValue::Scalar(CypherValue::String(s)) => Some(s.as_str()),
            _ => None,
        });
        from == Some("Alice") && to == Some("Charlie")
    });
    assert!(found_alice_charlie);
}

#[test]
fn query_var_length_with_label_filter() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})-[:KNOWS]->(c:Dog {name: "Rex"})"#,
    )
    .unwrap();

    // Match 1..2 hops where target is Person: Alice->Bob (1 hop), but NOT Alice->Rex (Rex is Dog)
    let result = g
        .cypher("MATCH (a:Person)-[*1..2]->(b:Person) RETURN a.name AS from, b.name AS to")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    // All targets should be Person
    for row in &rows {
        if let Some(ResultValue::Scalar(CypherValue::String(to))) = row.values.get("to") {
            assert!(to == "Alice" || to == "Bob");
        }
    }
}

#[test]
fn query_var_length_no_match() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})"#,
    )
    .unwrap();

    // No edges, so *1.. should return nothing
    let result = g
        .cypher("MATCH (a)-[*1..]->(b) RETURN a.name, b.name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}
