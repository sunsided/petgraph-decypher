//! Parser tests for Phase 4: Variable-length relationships.

use petgraph_cypher::{parse_cypher, Clause};

#[test]
fn parse_variable_length_bare_star() {
    let q = parse_cypher("MATCH (a)-[r*]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
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
    let q = parse_cypher("MATCH (a)-[r*3]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(3));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_variable_length_range() {
    let q = parse_cypher("MATCH (a)-[r*1..5]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(1));
    assert_eq!(vl.max, Some(5));
}

#[test]
fn parse_variable_length_min_only() {
    let q = parse_cypher("MATCH (a)-[r*2..]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(2));
    assert!(vl.max.is_none());
}

#[test]
fn parse_variable_length_max_only() {
    let q = parse_cypher("MATCH (a)-[r*..3]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    let vl = rel.variable_length.unwrap();
    assert_eq!(vl.min, Some(0));
    assert_eq!(vl.max, Some(3));
}

#[test]
fn parse_variable_length_with_type() {
    let q = parse_cypher("MATCH (a)-[r:KNOWS*1..3]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
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
    let q = parse_cypher("MATCH (a)-[*1..2]->(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.variable.is_none());
    assert!(rel.variable_length.is_some());
}

#[test]
fn parse_variable_length_left_directed() {
    let q = parse_cypher("MATCH (a)<-[r*1..3]-(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.variable_length.is_some());
}

#[test]
fn parse_variable_length_undirected() {
    let q = parse_cypher("MATCH (a)-[r*1..2]-(b) RETURN a, b").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert!(rel.variable_length.is_some());
}

#[test]
fn parse_variable_length_in_chain() {
    let q = parse_cypher("MATCH (a)-[r1*1..2]->(b)-[r2]->(c) RETURN a, b, c").unwrap();
    let Clause::Match { patterns, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert_eq!(patterns[0].rels.len(), 2);
    let (rel1, _) = &patterns[0].rels[0];
    let (rel2, _) = &patterns[0].rels[1];
    assert!(rel1.variable_length.is_some());
    assert!(rel2.variable_length.is_none());
}
