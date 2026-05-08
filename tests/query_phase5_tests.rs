//! Integration tests for Phase 5: Relationship type alternation.

use petgraph_cypher::{
    build_graph_from_cypher, CypherValue, PetgraphCypher, QueryResult, ResultValue, Row,
};

fn collect_rows(result: QueryResult) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Type alternation parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_type_alternation_two() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

#[test]
fn parse_type_alternation_three() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:A|B|C]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["A", "B", "C"]);
}

#[test]
fn parse_type_alternation_with_variable() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:KNOWS|LIKES]->(b) RETURN r, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.variable.as_deref(), Some("r"));
    assert_eq!(rel.rel_types, vec!["KNOWS", "LIKES"]);
}

#[test]
fn parse_type_alternation_with_variable_length() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS*1..3]->(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(1));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_type_alternation_left_directed() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)<-[r:KNOWS|FRIENDS]-(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

#[test]
fn parse_type_alternation_undirected() {
    let q = petgraph_cypher::parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS]-(b) RETURN a, b").unwrap();
    let petgraph_cypher::Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

// ---------------------------------------------------------------------------
// Type alternation execution
// ---------------------------------------------------------------------------

#[test]
fn query_type_alternation_match_any() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})
           CREATE (a)-[:FRIENDS]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    // Match either KNOWS or FRIENDS
    let result = g
        .cypher("MATCH (a:Person)-[r:KNOWS|FRIENDS]->(b:Person) RETURN a.name AS from, b.name AS to")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_type_alternation_match_partial() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})
           CREATE (a)-[:HATES]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    // Match KNOWS or FRIENDS — only KNOWS should match
    let result = g
        .cypher("MATCH (a:Person)-[r:KNOWS|FRIENDS]->(b:Person) RETURN a.name AS from, b.name AS to")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(to)) = rows[0].values.get("to").unwrap() {
        assert_eq!(to, "Bob");
    } else {
        panic!("expected string");
    }
}

#[test]
fn query_type_alternation_match_none() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:HATES]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Match KNOWS or FRIENDS — neither should match
    let result = g
        .cypher("MATCH (a:Person)-[r:KNOWS|FRIENDS]->(b:Person) RETURN a.name, b.name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}
