//! Integration tests for the Cypher parser.

use petgraph_cypher::{
    ast::Clause, parse_cypher, CypherError, CypherValue, Expression, RelDirection, WhereExpr,
};

#[test]
fn parse_create_single_node() {
    let q = parse_cypher("CREATE (n:Person {name: \"Alice\"})").unwrap();
    assert_eq!(q.clauses.len(), 1);
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    assert_eq!(patterns.len(), 1);
    let node = &patterns[0].start;
    assert_eq!(node.variable.as_deref(), Some("n"));
    assert_eq!(node.labels, vec!["Person"]);
    assert_eq!(
        node.properties.get("name"),
        Some(&CypherValue::String("Alice".into()))
    );
}

#[test]
fn parse_create_two_nodes_relationship() {
    let q = parse_cypher(r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#)
        .unwrap();
    assert_eq!(q.clauses.len(), 1);
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let path = &patterns[0];
    assert_eq!(path.start.variable.as_deref(), Some("a"));
    assert_eq!(path.rels.len(), 1);
    let (rel, target) = &path.rels[0];
    assert_eq!(rel.rel_type.as_deref(), Some("KNOWS"));
    assert_eq!(rel.direction, RelDirection::Right);
    assert_eq!(target.variable.as_deref(), Some("b"));
}

#[test]
fn parse_create_left_directed_relationship() {
    let q = parse_cypher("CREATE (a)<-[:LIKES]-(b)").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.rel_type.as_deref(), Some("LIKES"));
    assert_eq!(rel.direction, RelDirection::Both);
}

#[test]
fn parse_create_undirected_relationship() {
    let q = parse_cypher("CREATE (a)-[:PEER]-(b)").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.direction, RelDirection::Both);
}

#[test]
fn parse_create_multiple_labels() {
    let q = parse_cypher("CREATE (n:Person:Employee)").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let node = &patterns[0].start;
    assert_eq!(node.labels, vec!["Person", "Employee"]);
}

#[test]
fn parse_create_integer_and_float_properties() {
    let q = parse_cypher("CREATE (n:Item {count: 42, price: 9.99})").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let props = &patterns[0].start.properties;
    assert_eq!(props.get("count"), Some(&CypherValue::Integer(42)));
    assert_eq!(props.get("price"), Some(&CypherValue::Float(9.99)));
}

#[test]
fn parse_create_boolean_and_null_properties() {
    let q = parse_cypher("CREATE (n {active: true, deleted: false, memo: null})").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!("expected Create clause");
    };
    let props = &patterns[0].start.properties;
    assert_eq!(props.get("active"), Some(&CypherValue::Boolean(true)));
    assert_eq!(props.get("deleted"), Some(&CypherValue::Boolean(false)));
    assert_eq!(props.get("memo"), Some(&CypherValue::Null));
}

#[test]
fn parse_match_clause() {
    let q = parse_cypher("MATCH (n:Person)-[r:KNOWS]->(m)").unwrap();
    assert_eq!(q.clauses.len(), 1);
    let Clause::Match {
        patterns,
        where_clause,
    } = &q.clauses[0]
    else {
        panic!("expected Match clause");
    };
    assert!(where_clause.is_none());
    assert_eq!(patterns[0].start.labels, vec!["Person"]);
    let (rel, target) = &patterns[0].rels[0];
    assert_eq!(rel.rel_type.as_deref(), Some("KNOWS"));
    assert_eq!(target.variable.as_deref(), Some("m"));
}

#[test]
fn parse_match_with_where() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name = \"Alice\"").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match clause");
    };
    assert!(where_clause.is_some());
    let WhereExpr::Eq(expr, val) = where_clause.as_ref().unwrap() else {
        panic!("expected Eq expression");
    };
    assert_eq!(*expr, Expression::Property("n".into(), "name".into()));
    assert_eq!(*val, CypherValue::String("Alice".into()));
}

#[test]
fn parse_merge_clause() {
    let q = parse_cypher("MERGE (n:Person {name: \"Alice\"})").unwrap();
    assert_eq!(q.clauses.len(), 1);
    assert!(matches!(q.clauses[0], Clause::Merge { .. }));
}

#[test]
fn parse_return_clause() {
    let q = parse_cypher("MATCH (n) RETURN n, n.name AS name").unwrap();
    let Clause::Return { items } = &q.clauses[1] else {
        panic!("expected Return clause");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].expression, Expression::Variable("n".into()));
    assert!(items[0].alias.is_none());
    assert_eq!(
        items[1].expression,
        Expression::Property("n".into(), "name".into())
    );
    assert_eq!(items[1].alias.as_deref(), Some("name"));
}

#[test]
fn parse_delete_clause() {
    let q = parse_cypher("MATCH (n) DELETE n").unwrap();
    let Clause::Delete {
        variables,
        detach: false,
    } = &q.clauses[1]
    else {
        panic!("expected Delete clause");
    };
    assert_eq!(variables, &["n"]);
}

#[test]
fn parse_detach_delete_clause() {
    let q = parse_cypher("MATCH (n) DETACH DELETE n").unwrap();
    let Clause::Delete {
        variables,
        detach: true,
    } = &q.clauses[1]
    else {
        panic!("expected Delete clause with detach=true");
    };
    assert_eq!(variables, &["n"]);
}

#[test]
fn parse_multiple_clauses() {
    let q = parse_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           RETURN a, b"#,
    )
    .unwrap();
    assert_eq!(q.clauses.len(), 3);
}

#[test]
fn parse_semicolon_separated_statements() {
    let q = parse_cypher("CREATE (a:Person {name: \"Alice\"}); CREATE (b:Person {name: \"Bob\"})")
        .unwrap();
    assert_eq!(q.clauses.len(), 2);
}

#[test]
fn parse_simple_arrow_relationship() {
    let q = parse_cypher("CREATE (a)-->(b)").unwrap();
    let Clause::Create { patterns } = &q.clauses[0] else {
        panic!();
    };
    let (rel, _) = &patterns[0].rels[0];
    assert_eq!(rel.direction, RelDirection::Right);
    assert!(rel.rel_type.is_none());
}

#[test]
fn parse_error_on_invalid_input() {
    let result = parse_cypher("THIS IS NOT VALID CYPHER !!!");
    assert!(matches!(result, Err(CypherError::ParseError(_))));
}
