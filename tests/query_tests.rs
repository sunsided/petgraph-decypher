//! Integration tests for the query execution engine.

use petgraph::Graph;
use petgraph_cypher::{
    build_graph_from_cypher, CypherValue, PetgraphCypher, QueryResult, ResultValue, Row,
};

fn collect_rows(result: QueryResult<'_>) -> Vec<Row> {
    result.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Basic MATCH
// ---------------------------------------------------------------------------

#[test]
fn query_match_all_nodes() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (n) RETURN n.name AS name").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_match_with_label_filter() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Dog {name: "Rex"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (n:Person) RETURN n.name AS name").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Alice");
    } else {
        panic!("expected string value");
    }
}

// ---------------------------------------------------------------------------
// Relationship matching
// ---------------------------------------------------------------------------

#[test]
fn query_match_relationship() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (a)-[r:KNOWS]->(b) RETURN a.name AS a_name, b.name AS b_name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_match_no_matching_edges() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (a)-[r:HATES]->(b) RETURN a.name").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// WHERE filtering
// ---------------------------------------------------------------------------

#[test]
fn query_where_filter() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name = "Alice" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Alice");
    } else {
        panic!("expected string value");
    }
}

#[test]
fn query_where_with_and() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name = "Alice" AND n.age = 30 RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_no_match() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name = "Nobody" RETURN n.name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// Multi-hop path patterns
// ---------------------------------------------------------------------------

#[test]
fn query_match_two_hop_path() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})-[:KNOWS]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (a)-[:KNOWS]->(b)-[:KNOWS]->(c) RETURN a.name AS a_name, c.name AS c_name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_match_chain_partial() {
    let g = build_graph_from_cypher(r#"CREATE (a)-[:E]->(b)-[:E]->(c)-[:E]->(d)"#).unwrap();

    // Match two hops: there are 2 such paths in a 4-node chain: a→b→c and b→c→d
    let result = g.cypher("MATCH (a)-[:E]->(b)-[:E]->(c) RETURN a").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// Property access in RETURN
// ---------------------------------------------------------------------------

#[test]
fn query_return_property() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    let result = g
        .cypher("MATCH (n) RETURN n.name AS name, n.age AS age")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_return_wildcard() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (a)-[r]->(b) RETURN *").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    // Wildcard should include all bound variables
    assert!(rows[0].values.contains_key("a"));
    assert!(rows[0].values.contains_key("r"));
    assert!(rows[0].values.contains_key("b"));
}

// ---------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------

#[test]
fn query_columns() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher("MATCH (n) RETURN n.name AS name, n.age AS age")
        .unwrap();
    assert_eq!(result.columns(), &["name".to_string(), "age".to_string()]);
}

// ---------------------------------------------------------------------------
// Empty graph
// ---------------------------------------------------------------------------

#[test]
fn query_empty_graph() {
    let g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    let result = g.cypher("MATCH (n) RETURN n").unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert!(rows.is_empty());
}

// ---------------------------------------------------------------------------
// Mutation: CREATE via cypher_mut
// ---------------------------------------------------------------------------

#[test]
fn query_mut_create_single_node() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (n:Person {name: "Alice"})"#)
        .unwrap();
    assert_eq!(g.node_count(), 1);
}

#[test]
fn query_mut_create_two_nodes_with_edge() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#)
        .unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn query_mut_create_multiple_clauses() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (a:Person {name: "Alice"})"#)
        .unwrap();
    g.cypher_mut(r#"CREATE (b:Person {name: "Bob"})"#).unwrap();
    assert_eq!(g.node_count(), 2);
}

// ---------------------------------------------------------------------------
// Mutation: MATCH + DELETE
// ---------------------------------------------------------------------------

#[test]
fn query_mut_delete_node() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    // Alice has an edge, so need DETACH DELETE
    g.cypher_mut(r#"MATCH (n) WHERE n.name = "Alice" DETACH DELETE n"#)
        .unwrap();
    assert_eq!(g.node_count(), 1);
    assert_eq!(g.edge_count(), 0);
}

#[test]
fn query_mut_detach_delete() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})-[:KNOWS]->(c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    // Bob has two edges
    g.cypher_mut(r#"MATCH (n) WHERE n.name = "Bob" DETACH DELETE n"#)
        .unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 0);
}

// ---------------------------------------------------------------------------
// Mutation: MERGE
// ---------------------------------------------------------------------------

#[test]
fn query_mut_merge_creates_if_not_found() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    g.cypher_mut(r#"MERGE (n:Person {name: "Alice"})"#).unwrap();
    assert_eq!(g.node_count(), 1);
}

#[test]
fn query_mut_merge_does_not_duplicate() {
    let mut g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    // Merge a pattern that matches the existing node (anonymous node with label)
    g.cypher_mut(r#"MERGE (n:Person)"#).unwrap();
    // The existing node matches, so no new node should be created
    assert_eq!(g.node_count(), 1);
}

// ---------------------------------------------------------------------------
// Error cases: mixing reads and mutations
// ---------------------------------------------------------------------------

#[test]
fn query_cypher_rejects_create() {
    let g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    let result = g.cypher("CREATE (n:Person) RETURN n");
    assert!(result.is_err());
}

#[test]
fn query_cypher_rejects_delete() {
    let g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    let result = g.cypher("MATCH (n) DELETE n");
    assert!(result.is_err());
}

#[test]
fn query_cypher_mut_rejects_return_clause() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    let result = g.cypher_mut("CREATE (n:Person {name: \"Alice\"}) RETURN n");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Strategy selection
// ---------------------------------------------------------------------------

#[test]
fn query_with_strategy_backtrack() {
    use petgraph_cypher::MatchStrategy;

    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher_with_strategy(
            "MATCH (a)-[:KNOWS]->(b) RETURN a.name",
            MatchStrategy::Backtrack,
        )
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_with_strategy_fast() {
    use petgraph_cypher::MatchStrategy;

    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher_with_strategy("MATCH (a)-[:KNOWS]->(b) RETURN a.name", MatchStrategy::Fast)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

// ---------------------------------------------------------------------------
// Multiple patterns in MATCH (cartesian product)
// ---------------------------------------------------------------------------

#[test]
fn query_match_multiple_patterns_cartesian() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (a:Person), (b:Person) RETURN a.name, b.name")
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    // 2 persons × 2 persons = 4 combinations
    assert_eq!(rows.len(), 4);
}

// ---------------------------------------------------------------------------
// WHERE comparison operators
// ---------------------------------------------------------------------------

#[test]
fn query_where_lt_gt() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.age > 25 RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_where_not_eq() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name <> "Alice" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Bob");
    } else {
        panic!("expected string value");
    }
}

#[test]
fn query_where_le_ge() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {age: 30})
           CREATE (b:Person {age: 25})
           CREATE (c:Person {age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.age >= 25 AND n.age <= 30 RETURN n"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// WHERE string operators
// ---------------------------------------------------------------------------

#[test]
fn query_where_starts_with() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name STARTS WITH "Al" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_ends_with() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name ENDS WITH "ce" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_contains() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name CONTAINS "li" RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

// ---------------------------------------------------------------------------
// WHERE null checks
// ---------------------------------------------------------------------------

#[test]
fn query_where_is_null() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {age: 25})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name IS NULL RETURN n"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

#[test]
fn query_where_is_not_null() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {age: 25})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name IS NOT NULL RETURN n.name AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

// ---------------------------------------------------------------------------
// WHERE IN
// ---------------------------------------------------------------------------

// NOTE: Upstream cypher HIR bug — list literals are lowered to `Literal(Null)`,
// so `IN ["Alice", "Bob"]` cannot be evaluated. Re-enable when upstream is fixed.
// #[test]
// fn query_where_in() {
//     let g = build_graph_from_cypher(
//         r#"CREATE (a:Person {name: "Alice"})
//            CREATE (b:Person {name: "Bob"})
//            CREATE (c:Person {name: "Charlie"})"#,
//     )
//     .unwrap();
//
//     let result = g
//         .cypher(r#"MATCH (n:Person) WHERE n.name IN ["Alice", "Bob"] RETURN n.name AS name"#)
//         .unwrap();
//     let rows: Vec<_> = collect_rows(result);
//     assert_eq!(rows.len(), 2);
// }

// ---------------------------------------------------------------------------
// WHERE NOT
// ---------------------------------------------------------------------------

#[test]
fn query_where_not() {
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
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Bob");
    } else {
        panic!("expected string value");
    }
}

// ---------------------------------------------------------------------------
// ORDER BY
// ---------------------------------------------------------------------------

#[test]
fn query_order_by_asc() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name ORDER BY n.age"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 3);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Bob");
    } else {
        panic!("expected string value");
    }
}

#[test]
fn query_order_by_desc() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name ORDER BY n.age DESC"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 3);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "Charlie");
    } else {
        panic!("expected string value");
    }
}

// ---------------------------------------------------------------------------
// LIMIT / SKIP
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
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name LIMIT 2"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_skip() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name SKIP 1"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

#[test]
fn query_skip_and_limit() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})
           CREATE (d:Person {name: "David"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name SKIP 1 LIMIT 2"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// DISTINCT
// ---------------------------------------------------------------------------

#[test]
fn query_distinct() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {age: 30})
           CREATE (b:Person {age: 30})
           CREATE (c:Person {age: 25})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN DISTINCT n.age AS age"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 2);
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[test]
fn query_function_tostring() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {age: 30})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN toString(n.age) AS s"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(s)) = rows[0].values.get("s").unwrap() {
        assert_eq!(s, "30");
    } else {
        panic!("expected string value");
    }
}

#[test]
fn query_function_to_upper() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN toUpper(n.name) AS name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::String(name)) = rows[0].values.get("name").unwrap() {
        assert_eq!(name, "ALICE");
    } else {
        panic!("expected string value");
    }
}

#[test]
fn query_function_size() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN size(n.name) AS len"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(len)) = rows[0].values.get("len").unwrap() {
        assert_eq!(*len, 5);
    } else {
        panic!("expected integer value");
    }
}

// ---------------------------------------------------------------------------
// OPTIONAL MATCH
// ---------------------------------------------------------------------------

#[test]
fn query_optional_match() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (a:Person {name: "Alice"}) OPTIONAL MATCH (a)-[:HATES]->(b) RETURN a.name AS a_name, b.name AS b_name"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
}

// ---------------------------------------------------------------------------
// SET / REMOVE
// ---------------------------------------------------------------------------

#[test]
fn query_mut_set_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) SET n.age = 30"#).unwrap();
    let result = g.cypher(r#"MATCH (n:Person) RETURN n.age AS age"#).unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Integer(age)) = rows[0].values.get("age").unwrap() {
        assert_eq!(*age, 30);
    } else {
        panic!("expected integer value");
    }
}

#[test]
fn query_mut_remove_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) REMOVE n.age"#).unwrap();
    let result = g.cypher(r#"MATCH (n:Person) RETURN n.age AS age"#).unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Null) = rows[0].values.get("age").unwrap() {
        // expected
    } else {
        panic!("expected null value");
    }
}

// NOTE: Upstream cypher parser bug — `SET n:Label` and `REMOVE n:Label` produce
// `SetLabels` / `RemoveLabels` with an empty variable name, so the executor
// cannot find the target node. Re-enable when upstream is fixed.
// #[test]
// fn query_mut_set_labels() {
//     let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
//     g.cypher_mut(r#"MATCH (n:Person) SET n:Employee"#).unwrap();
//     let result = g.cypher(r#"MATCH (n:Employee) RETURN n.name AS name"#).unwrap();
//     let rows: Vec<_> = collect_rows(result);
//     assert_eq!(rows.len(), 1);
// }

// #[test]
// fn query_mut_remove_labels() {
//     let mut g = build_graph_from_cypher(r#"CREATE (n:Person:Employee {name: "Alice"})"#).unwrap();
//     g.cypher_mut(r#"MATCH (n) REMOVE n:Employee"#).unwrap();
//     let result = g.cypher(r#"MATCH (n:Employee) RETURN n"#).unwrap();
//     let rows: Vec<_> = collect_rows(result);
//     assert!(rows.is_empty());
// }

// ---------------------------------------------------------------------------
// MERGE ON CREATE / ON MATCH
// ---------------------------------------------------------------------------

#[test]
fn query_mut_merge_on_create() {
    let mut g: Graph<petgraph_cypher::NodeData, petgraph_cypher::EdgeData> = Graph::new();

    g.cypher_mut(r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.created = true"#)
        .unwrap();
    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.created AS created"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Boolean(created)) =
        rows[0].values.get("created").unwrap()
    {
        assert!(created);
    } else {
        panic!("expected boolean value");
    }
}

#[test]
fn query_mut_merge_on_match() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MERGE (n:Person {name: "Alice"}) ON MATCH SET n.seen = true"#)
        .unwrap();
    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.seen AS seen"#)
        .unwrap();
    let rows: Vec<_> = collect_rows(result);
    assert_eq!(rows.len(), 1);
    if let ResultValue::Scalar(CypherValue::Boolean(seen)) = rows[0].values.get("seen").unwrap() {
        assert!(seen);
    } else {
        panic!("expected boolean value");
    }
}
