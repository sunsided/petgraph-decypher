//! Integration tests for new Phase 1+ parser features.

use petgraph_cypher::{
    ast::{Clause, Expression, OrderDirection, WhereExpr},
    parse_cypher,
};

// ---------------------------------------------------------------------------
// Rich WHERE operators
// ---------------------------------------------------------------------------

#[test]
fn parse_where_neq() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name <> \"Alice\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Neq(_, _))));
}

#[test]
fn parse_where_neq_bang() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name != \"Alice\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Neq(_, _))));
}

#[test]
fn parse_where_lt() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.age < 30 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Lt(_, _))));
}

#[test]
fn parse_where_gt() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.age > 30 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Gt(_, _))));
}

#[test]
fn parse_where_lte() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.age <= 30 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Lte(_, _))));
}

#[test]
fn parse_where_gte() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.age >= 30 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Gte(_, _))));
}

#[test]
fn parse_where_is_null() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name IS NULL RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::IsNull(_))));
}

#[test]
fn parse_where_is_not_null() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name IS NOT NULL RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::IsNotNull(_))));
}

#[test]
fn parse_where_starts_with() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name STARTS WITH \"Al\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::StartsWith(_, _))));
}

#[test]
fn parse_where_ends_with() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name ENDS WITH \"ice\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::EndsWith(_, _))));
}

#[test]
fn parse_where_contains() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name CONTAINS \"li\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Contains(_, _))));
}

#[test]
fn parse_where_not() {
    let q = parse_cypher("MATCH (n:Person) WHERE NOT n.name = \"Alice\" RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Not(_))));
}

#[test]
fn parse_where_or() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name = \"Alice\" OR n.name = \"Bob\" RETURN n")
        .unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::Or(_, _))));
}

#[test]
fn parse_where_and_or_precedence() {
    // AND binds tighter than OR: (a = 1 AND b = 2) OR c = 3
    let q = parse_cypher("MATCH (n) WHERE n.a = 1 AND n.b = 2 OR n.c = 3 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    // Top-level should be Or
    assert!(matches!(where_clause, Some(WhereExpr::Or(_, _))));
}

#[test]
fn parse_where_in() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name IN [\"Alice\", \"Bob\"] RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    assert!(matches!(where_clause, Some(WhereExpr::In(_, _))));
}

#[test]
fn parse_where_parenthesized() {
    let q = parse_cypher("MATCH (n) WHERE (n.a = 1 OR n.b = 2) AND n.c = 3 RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    // Top-level should be And
    assert!(matches!(where_clause, Some(WhereExpr::And(_, _))));
}

// ---------------------------------------------------------------------------
// Arithmetic expressions
// ---------------------------------------------------------------------------

#[test]
fn parse_arithmetic_add() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.age + 5 AS adjusted_age").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Add(_, _)));
}

#[test]
fn parse_arithmetic_sub() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.age - 5 AS adjusted_age").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Sub(_, _)));
}

#[test]
fn parse_arithmetic_mul() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.age * 2 AS doubled").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Mul(_, _)));
}

#[test]
fn parse_arithmetic_div() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.age / 2 AS half").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Div(_, _)));
}

#[test]
fn parse_arithmetic_precedence() {
    // 1 + 2 * 3 should be 1 + (2 * 3), not (1 + 2) * 3
    let q = parse_cypher("MATCH (n) RETURN n.a + n.b * n.c AS result").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    // Top-level should be Add, right side should be Mul
    let Expression::Add(_, right) = &items[0].expression else {
        panic!("expected Add at top level");
    };
    assert!(matches!(**right, Expression::Mul(_, _)));
}

// ---------------------------------------------------------------------------
// Nested property access
// ---------------------------------------------------------------------------

#[test]
fn parse_nested_property_access() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.address.city AS city").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(
        items[0].expression,
        Expression::NestedProperty { .. }
    ));
}

// ---------------------------------------------------------------------------
// Function calls
// ---------------------------------------------------------------------------

#[test]
fn parse_function_call() {
    let q = parse_cypher("MATCH (n:Person) RETURN count(n) AS cnt").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(
        items[0].expression,
        Expression::FunctionCall { .. }
    ));
}

#[test]
fn parse_function_call_multiple_args() {
    let q = parse_cypher("MATCH (n:Person) RETURN coalesce(n.name, \"Unknown\") AS name").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    match &items[0].expression {
        Expression::FunctionCall { name, args } => {
            assert_eq!(name, "coalesce");
            assert_eq!(args.len(), 2);
        }
        _ => panic!("expected FunctionCall"),
    }
}

// ---------------------------------------------------------------------------
// List literals
// ---------------------------------------------------------------------------

#[test]
fn parse_list_literal() {
    let q = parse_cypher("RETURN [1, 2, 3] AS list").unwrap();
    let Clause::Return { items, .. } = &q.clauses[0] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::List(_)));
}

#[test]
fn parse_empty_list() {
    let q = parse_cypher("RETURN [] AS empty").unwrap();
    let Clause::Return { items, .. } = &q.clauses[0] else {
        panic!("expected Return");
    };
    match &items[0].expression {
        Expression::List(exprs) => assert!(exprs.is_empty()),
        _ => panic!("expected List"),
    }
}

// ---------------------------------------------------------------------------
// Parameter references
// ---------------------------------------------------------------------------

#[test]
fn parse_parameter_ref() {
    let q = parse_cypher("MATCH (n:Person) WHERE n.name = $name RETURN n").unwrap();
    let Clause::Match { where_clause, .. } = &q.clauses[0] else {
        panic!("expected Match");
    };
    // Parameter in WHERE comparison is parsed; value is CypherValue::Null at parse time
    // (actual resolution happens at runtime)
    assert!(
        matches!(where_clause, Some(WhereExpr::Eq(expr, val)) if matches!(&expr, Expression::Property(a, b) if a == "n" && b == "name"))
    );
}

#[test]
fn parse_parameter_in_return() {
    let q = parse_cypher("MATCH (n:Person) RETURN n, $threshold AS threshold").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[1].expression, Expression::Parameter(_)));
}

// ---------------------------------------------------------------------------
// Backtick-quoted identifiers
// ---------------------------------------------------------------------------

#[test]
fn parse_backtick_identifier() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.`first-name` AS name").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    // Should be property access with backtick key
    match &items[0].expression {
        Expression::Property(_, prop) => assert_eq!(prop, "first-name"),
        _ => panic!("expected Property"),
    }
}

// ---------------------------------------------------------------------------
// CASE expression
// ---------------------------------------------------------------------------

#[test]
fn parse_case_expression() {
    let q = parse_cypher(
        "MATCH (n) RETURN CASE WHEN n.age > 18 THEN 'adult' ELSE 'minor' END AS category",
    )
    .unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Case { .. }));
}

#[test]
fn parse_case_multiple_when() {
    let q = parse_cypher("MATCH (n) RETURN CASE WHEN n.age < 13 THEN 'child' WHEN n.age < 18 THEN 'teen' ELSE 'adult' END AS category").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    match &items[0].expression {
        Expression::Case { arms, .. } => assert_eq!(arms.len(), 2),
        _ => panic!("expected Case"),
    }
}

// ---------------------------------------------------------------------------
// Parenthesized expressions
// ---------------------------------------------------------------------------

#[test]
fn parse_parenthesized_expression() {
    let q = parse_cypher("MATCH (n) RETURN (n.a + n.b) * 2 AS result").unwrap();
    let Clause::Return { items, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    // (n.a + n.b) * 2 => Mul at top level
    assert!(matches!(items[0].expression, Expression::Mul(_, _)));
}

// ---------------------------------------------------------------------------
// Map literal as expression
// ---------------------------------------------------------------------------

#[test]
fn parse_map_literal() {
    let q = parse_cypher("RETURN {name: \"Alice\", age: 30} AS person").unwrap();
    let Clause::Return { items, .. } = &q.clauses[0] else {
        panic!("expected Return");
    };
    assert!(matches!(items[0].expression, Expression::Map(_)));
}

// ---------------------------------------------------------------------------
// RETURN DISTINCT
// ---------------------------------------------------------------------------

#[test]
fn parse_return_distinct() {
    let q = parse_cypher("MATCH (n:Person) RETURN DISTINCT n.name").unwrap();
    let Clause::Return { distinct, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(*distinct);
}

#[test]
fn parse_return_not_distinct() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.name").unwrap();
    let Clause::Return { distinct, .. } = &q.clauses[1] else {
        panic!("expected Return");
    };
    assert!(!*distinct);
}

// ---------------------------------------------------------------------------
// ORDER BY, SKIP, LIMIT parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_order_by() {
    let q = parse_cypher("MATCH (n:Person) RETURN n.name ORDER BY n.name").unwrap();
    assert_eq!(q.order_by.len(), 1);
    assert!(q.skip.is_none());
    assert!(q.limit.is_none());
}

#[test]
fn parse_order_by_desc() {
    let q = parse_cypher("MATCH (n) RETURN n.name ORDER BY n.name DESC").unwrap();
    assert_eq!(q.order_by.len(), 1);
    assert_eq!(q.order_by[0].direction, OrderDirection::Descending);
}

#[test]
fn parse_order_by_multiple() {
    let q = parse_cypher("MATCH (n) RETURN n.name, n.age ORDER BY n.name ASC, n.age DESC").unwrap();
    assert_eq!(q.order_by.len(), 2);
}

#[test]
fn parse_skip() {
    let q = parse_cypher("MATCH (n) RETURN n SKIP 5").unwrap();
    assert_eq!(q.skip, Some(5));
    assert!(q.limit.is_none());
}

#[test]
fn parse_limit() {
    let q = parse_cypher("MATCH (n) RETURN n LIMIT 10").unwrap();
    assert!(q.skip.is_none());
    assert_eq!(q.limit, Some(10));
}

#[test]
fn parse_skip_and_limit() {
    let q = parse_cypher("MATCH (n) RETURN n SKIP 5 LIMIT 10").unwrap();
    assert_eq!(q.skip, Some(5));
    assert_eq!(q.limit, Some(10));
}

#[test]
fn parse_order_by_skip_limit() {
    let q = parse_cypher("MATCH (n) RETURN n.name ORDER BY n.name SKIP 5 LIMIT 10").unwrap();
    assert_eq!(q.order_by.len(), 1);
    assert_eq!(q.skip, Some(5));
    assert_eq!(q.limit, Some(10));
}

// ---------------------------------------------------------------------------
// SET clause
// ---------------------------------------------------------------------------

#[test]
fn parse_set_property() {
    let q = parse_cypher("MATCH (n) SET n.name = \"Alice\"").unwrap();
    assert_eq!(q.clauses.len(), 2);
    assert!(matches!(q.clauses[1], Clause::SetProperty { .. }));
}

#[test]
fn parse_set_merge() {
    let q = parse_cypher("MATCH (n) SET n += {name: \"Alice\"}").unwrap();
    assert_eq!(q.clauses.len(), 2);
    assert!(matches!(q.clauses[1], Clause::SetMerge { .. }));
}

#[test]
fn parse_set_add_labels() {
    let q = parse_cypher("MATCH (n) SET n:Person:Employee").unwrap();
    assert_eq!(q.clauses.len(), 2);
    assert!(matches!(q.clauses[1], Clause::SetAddLabels { .. }));
}

// ---------------------------------------------------------------------------
// REMOVE clause
// ---------------------------------------------------------------------------

#[test]
fn parse_remove_property() {
    let q = parse_cypher("MATCH (n) REMOVE n.name").unwrap();
    assert_eq!(q.clauses.len(), 2);
    assert!(matches!(q.clauses[1], Clause::RemoveProperty { .. }));
}

#[test]
fn parse_remove_labels() {
    let q = parse_cypher("MATCH (n) REMOVE n:Person:Employee").unwrap();
    assert_eq!(q.clauses.len(), 2);
    assert!(matches!(q.clauses[1], Clause::RemoveLabels { .. }));
}
