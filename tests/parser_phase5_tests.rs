//! Parser tests for Phase 5: Relationship type alternation.

use petgraph_cypher::{parse_cypher, Clause};

#[test]
fn parse_type_alternation_two() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

#[test]
fn parse_type_alternation_three() {
    let q = parse_cypher("MATCH (a)-[r:A|B|C]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["A", "B", "C"]);
}

#[test]
fn parse_type_alternation_with_variable() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS|LIKES]->(b) RETURN r, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.variable.as_deref(), Some("r"));
    assert_eq!(rel.rel_types, vec!["KNOWS", "LIKES"]);
}

#[test]
fn parse_type_alternation_with_variable_length() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS*1..3]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
    assert!(rel.variable_length.is_some());
}

#[test]
fn parse_type_alternation_left_directed() {
    let q = parse_cypher("MATCH (a)<-[r:KNOWS|FRIENDS]-(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

#[test]
fn parse_type_alternation_undirected() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS|FRIENDS]-(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS", "FRIENDS"]);
}

#[test]
fn parse_single_type_still_works() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_types, vec!["KNOWS"]);
}

#[test]
fn parse_no_type_still_works() {
    let q = parse_cypher("MATCH (a)-[r]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.rel_types.is_empty());
}
