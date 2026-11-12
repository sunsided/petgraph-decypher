//! Integration tests for the query execution engine.

use petgraph::Graph;
use petgraph::stable_graph::StableGraph;
use petgraph_decypher::{
    CypherEdge, CypherEdgeMut, CypherError, CypherNode, CypherNodeMut, EdgeData, NodeData,
    Parameters, PetgraphCypher, PetgraphCypherMut, Properties, Query, QueryResult, Value,
    build_graph_from_cypher, build_graph_from_cypher_typed,
};
use std::collections::{BTreeSet, HashMap};

/// The values of column `name`, one per row.
fn column(result: &QueryResult, name: &str) -> Vec<Value> {
    assert!(
        result.columns().iter().any(|c| c == name),
        "no column {name:?} in {:?}",
        result.columns()
    );
    result.column(name).cloned().collect()
}

/// The string values of column `name`, one per row.
fn strings(result: &QueryResult, name: &str) -> Vec<String> {
    column(result, name)
        .into_iter()
        .map(|v| match v {
            Value::String(s) => s,
            other => panic!("expected a string in column {name:?}, got {other:?}"),
        })
        .collect()
}

fn params<const N: usize>(entries: [(&str, Value); N]) -> Parameters {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct CustomNode {
    labels: Vec<String>,
    properties: HashMap<String, Value>,
}

impl CypherNode for CustomNode {
    fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().map(String::as_str)
    }

    fn property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    fn properties(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl CypherNodeMut for CustomNode {
    fn from_cypher(_variable: Option<&str>, labels: Vec<String>, properties: Properties) -> Self {
        Self {
            labels,
            properties: properties.into_iter().collect(),
        }
    }

    fn add_label(&mut self, label: &str) {
        if !self.has_label(label) {
            self.labels.push(label.to_owned());
        }
    }

    fn remove_label(&mut self, label: &str) {
        self.labels.retain(|l| l != label);
    }

    fn set_property(&mut self, key: &str, value: Value) {
        if value.is_null() {
            self.properties.remove(key);
        } else {
            self.properties.insert(key.to_owned(), value);
        }
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CustomEdge {
    rel_type: String,
    properties: HashMap<String, Value>,
}

impl CypherEdge for CustomEdge {
    fn rel_type(&self) -> &str {
        &self.rel_type
    }

    fn property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    fn properties(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl CypherEdgeMut for CustomEdge {
    fn from_cypher(_variable: Option<&str>, rel_type: String, properties: Properties) -> Self {
        Self {
            rel_type,
            properties: properties.into_iter().collect(),
        }
    }

    fn set_property(&mut self, key: &str, value: Value) {
        if value.is_null() {
            self.properties.remove(key);
        } else {
            self.properties.insert(key.to_owned(), value);
        }
    }

    fn clear_properties(&mut self) {
        self.properties.clear();
    }
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
    assert_eq!(result.len(), 2);
}

#[test]
fn query_match_all_nodes_with_custom_weights() {
    let g = build_graph_from_cypher_typed::<CustomNode, CustomEdge>(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (n:Person) RETURN n.name AS name").unwrap();
    assert_eq!(result.columns(), ["name"]);
    let mut names = strings(&result, "name");
    names.sort();
    assert_eq!(names, ["Alice", "Bob"]);
}

#[test]
fn query_match_relationship_with_custom_weights() {
    let g = build_graph_from_cypher_typed::<CustomNode, CustomEdge>(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g
        .cypher("MATCH (a)-[r:KNOWS]->(b) RETURN r.since AS since, b.name AS name")
        .unwrap();
    assert_eq!(result.columns(), ["since", "name"]);
    assert_eq!(column(&result, "since"), [Value::from(2020)]);
    assert_eq!(strings(&result, "name"), ["Bob"]);
}

#[test]
fn query_match_with_label_filter() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Dog {name: "Rex"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (n:Person) RETURN n.name AS name").unwrap();
    assert_eq!(strings(&result, "name"), ["Alice"]);
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
    assert_eq!(strings(&result, "a_name"), ["Alice"]);
    assert_eq!(strings(&result, "b_name"), ["Bob"]);
}

#[test]
fn query_match_no_matching_edges() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (a)-[r:HATES]->(b) RETURN a.name").unwrap();
    assert!(result.is_empty());
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
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
    assert_eq!(result.len(), 1);
}

#[test]
fn query_where_with_parameters() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})"#,
    )
    .unwrap();

    let params = params([("name", "Alice".into()), ("min_age", 30.into())]);
    let result = g
        .cypher_params(
            "MATCH (n:Person) WHERE n.name = $name AND n.age >= $min_age RETURN n.name AS name",
            &params,
        )
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice"]);
}

#[test]
fn query_where_no_match() {
    let g = build_graph_from_cypher(r#"CREATE (a:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name = "Nobody" RETURN n.name"#)
        .unwrap();
    assert!(result.is_empty());
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
    assert_eq!(strings(&result, "a_name"), ["Alice"]);
    assert_eq!(strings(&result, "c_name"), ["Charlie"]);
}

#[test]
fn query_match_chain_partial() {
    let g = build_graph_from_cypher(r#"CREATE (a)-[:E]->(b)-[:E]->(c)-[:E]->(d)"#).unwrap();

    // Match two hops: there are 2 such paths in a 4-node chain: a→b→c and b→c→d
    let result = g.cypher("MATCH (a)-[:E]->(b)-[:E]->(c) RETURN a").unwrap();
    assert_eq!(result.len(), 2);
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
    assert_eq!(column(&result, "age"), [Value::from(30)]);
}

#[test]
fn query_return_unaliased_property_column_name() {
    // openCypher names an unaliased column after the expression text.
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g.cypher("MATCH (n) RETURN n.name").unwrap();
    assert_eq!(result.columns(), ["n.name"]);
    assert_eq!(strings(&result, "n.name"), ["Alice"]);
}

#[test]
fn query_return_wildcard() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
    )
    .unwrap();

    let result = g.cypher("MATCH (a)-[r]->(b) RETURN *").unwrap();
    assert_eq!(result.len(), 1);
    // Wildcard should include all bound variables
    let row = result.rows().next().unwrap();
    let (Some(Value::Node(a)), Some(Value::Relationship(r)), Some(Value::Node(b))) =
        (row.get("a"), row.get("r"), row.get("b"))
    else {
        panic!("expected a, r, b columns, got {:?}", result.columns());
    };
    assert_eq!(g[*a].properties["name"], Value::from("Alice"));
    assert_eq!(g[*r].rel_type, "KNOWS");
    assert_eq!(g[*b].properties["name"], Value::from("Bob"));
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
    assert_eq!(result.columns(), ["name", "age"]);
}

// ---------------------------------------------------------------------------
// Empty graph
// ---------------------------------------------------------------------------

#[test]
fn query_empty_graph() {
    let g: Graph<NodeData, EdgeData> = Graph::new();

    let result = g.cypher("MATCH (n) RETURN n").unwrap();
    assert!(result.is_empty());
}

// ---------------------------------------------------------------------------
// Mutation: CREATE via cypher_mut
// ---------------------------------------------------------------------------

#[test]
fn query_mut_create_single_node() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (n:Person {name: "Alice"})"#)
        .unwrap();
    assert_eq!(g.node_count(), 1);
}

#[test]
fn query_mut_create_two_nodes_with_edge() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#)
        .unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn query_mut_create_multiple_clauses() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    g.cypher_mut(r#"CREATE (a:Person {name: "Alice"})"#)
        .unwrap();
    g.cypher_mut(r#"CREATE (b:Person {name: "Bob"})"#).unwrap();
    assert_eq!(g.node_count(), 2);
}

#[test]
fn query_mut_create_returns_rows() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    let result = g.cypher_mut("CREATE (n:X {v: 1}) RETURN n.v").unwrap();
    assert_eq!(result.columns(), ["n.v"]);
    assert_eq!(column(&result, "n.v"), [Value::from(1)]);
    assert_eq!(g.node_count(), 1);
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

const FIVE_NODES: &str =
    "CREATE (n0 {id: 0}), (n1 {id: 1}), (n2 {id: 2}), (n3 {id: 3}), (n4 {id: 4})
     CREATE (n0)-[:R]->(n1), (n1)-[:R]->(n3), (n3)-[:R]->(n4), (n4)-[:R]->(n2)";

fn id(node: &NodeData) -> i64 {
    match node.properties.get("id") {
        Some(Value::Integer(i)) => *i,
        other => panic!("expected integer id, got {other:?}"),
    }
}

#[test]
fn query_mut_delete_several_nodes_keeps_survivors_intact() {
    let mut g = build_graph_from_cypher(FIVE_NODES).unwrap();

    g.cypher_mut("MATCH (n) WHERE n.id IN [0, 2] DETACH DELETE n")
        .unwrap();

    let ids: BTreeSet<i64> = g.node_weights().map(id).collect();
    assert_eq!(ids, BTreeSet::from([1, 3, 4]));
    let edges: BTreeSet<(i64, i64)> = g
        .edge_indices()
        .map(|e| g.edge_endpoints(e).unwrap())
        .map(|(s, t)| (id(&g[s]), id(&g[t])))
        .collect();
    assert_eq!(edges, BTreeSet::from([(1, 3), (3, 4)]));
}

#[test]
fn query_mut_delete_several_nodes_on_stable_graph_keeps_indices() {
    let mut g: StableGraph<NodeData, EdgeData> = StableGraph::new();
    g.cypher_mut(FIVE_NODES).unwrap();
    let before: HashMap<i64, _> = g.node_indices().map(|n| (id(&g[n]), n)).collect();

    g.cypher_mut("MATCH (n) WHERE n.id IN [0, 2] DETACH DELETE n")
        .unwrap();

    let after: HashMap<i64, _> = g.node_indices().map(|n| (id(&g[n]), n)).collect();
    assert_eq!(after.len(), 3);
    for i in [1, 3, 4] {
        assert_eq!(after[&i], before[&i], "index of node {i} changed");
    }
    let edges: BTreeSet<(i64, i64)> = g
        .edge_indices()
        .map(|e| g.edge_endpoints(e).unwrap())
        .map(|(s, t)| (id(&g[s]), id(&g[t])))
        .collect();
    assert_eq!(edges, BTreeSet::from([(1, 3), (3, 4)]));
}

// ---------------------------------------------------------------------------
// Mutation: MERGE
// ---------------------------------------------------------------------------

#[test]
fn query_mut_merge_creates_if_not_found() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

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
    let g: Graph<NodeData, EdgeData> = Graph::new();

    let result = g.cypher("CREATE (n:Person) RETURN n");
    assert!(matches!(result, Err(CypherError::InvalidQuery(_))));
}

#[test]
fn query_cypher_rejects_delete() {
    let g: Graph<NodeData, EdgeData> = Graph::new();

    let result = g.cypher("MATCH (n) DELETE n");
    assert!(matches!(result, Err(CypherError::InvalidQuery(_))));
}

#[test]
fn query_cypher_mut_accepts_return_clause() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    let result = g
        .cypher_mut("CREATE (n:Person {name: \"Alice\"}) RETURN n")
        .unwrap();
    let [Value::Node(n)] = column(&result, "n")[..] else {
        panic!("expected one node row");
    };
    assert_eq!(g[n].properties["name"], Value::from("Alice"));
}

// ---------------------------------------------------------------------------
// Prepared queries
// ---------------------------------------------------------------------------

#[test]
fn query_prepared_executes_with_different_parameters() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})"#,
    )
    .unwrap();

    let query = Query::new("MATCH (n:Person) WHERE n.name = $name RETURN n.age AS age").unwrap();
    assert!(query.is_read_only());

    let alice = query
        .execute(&g, &params([("name", "Alice".into())]))
        .unwrap();
    assert_eq!(column(&alice, "age"), [Value::from(30)]);
    let bob = query
        .execute(&g, &params([("name", "Bob".into())]))
        .unwrap();
    assert_eq!(column(&bob, "age"), [Value::from(25)]);
}

#[test]
fn query_prepared_mutating_query_runs_with_execute_mut() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();
    let create = Query::new("CREATE (:Person {name: $name})").unwrap();
    assert!(!create.is_read_only());

    for name in ["Alice", "Bob"] {
        create
            .execute_mut(&mut g, &params([("name", name.into())]))
            .unwrap();
    }
    assert!(matches!(
        create.execute(&g, &params([("name", "Carol".into())])),
        Err(CypherError::InvalidQuery(_))
    ));

    let result = g
        .cypher("MATCH (p:Person) RETURN p.name AS name ORDER BY name")
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice", "Bob"]);
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
    assert_eq!(result.columns(), ["a.name", "b.name"]);
    // 2 persons × 2 persons = 4 combinations
    assert_eq!(result.len(), 4);
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
        .cypher(r#"MATCH (n:Person) WHERE n.age > 25 RETURN n.name AS name ORDER BY name"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice", "Charlie"]);
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
    assert_eq!(strings(&result, "name"), ["Bob"]);
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
    assert_eq!(result.len(), 2);
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
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
        .cypher(r#"MATCH (n:Person) WHERE n.name IS NULL RETURN n.age AS age"#)
        .unwrap();
    assert_eq!(column(&result, "age"), [Value::from(25)]);
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
    assert_eq!(strings(&result, "name"), ["Alice"]);
}

// ---------------------------------------------------------------------------
// WHERE IN
// ---------------------------------------------------------------------------

#[test]
fn query_where_in() {
    let g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})
           CREATE (c:Person {name: "Charlie"})"#,
    )
    .unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) WHERE n.name IN ["Alice", "Bob"] RETURN n.name AS name ORDER BY name"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice", "Bob"]);
}

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
    assert_eq!(strings(&result, "name"), ["Bob"]);
}

// ---------------------------------------------------------------------------
// ORDER BY
// ---------------------------------------------------------------------------

const THREE_AGES: &str = r#"CREATE (a:Person {name: "Alice", age: 30})
           CREATE (b:Person {name: "Bob", age: 25})
           CREATE (c:Person {name: "Charlie", age: 35})"#;

#[test]
fn query_order_by_asc() {
    let g = build_graph_from_cypher(THREE_AGES).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name ORDER BY n.age"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Bob", "Alice", "Charlie"]);
}

#[test]
fn query_order_by_desc() {
    let g = build_graph_from_cypher(THREE_AGES).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name ORDER BY n.age DESC"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Charlie", "Alice", "Bob"]);
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
    assert_eq!(result.len(), 2);
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
    assert_eq!(result.len(), 2);
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
        .cypher(r#"MATCH (n:Person) RETURN n.name AS name ORDER BY name SKIP 1 LIMIT 2"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Bob", "Charlie"]);
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
        .cypher(r#"MATCH (n:Person) RETURN DISTINCT n.age AS age ORDER BY age"#)
        .unwrap();
    assert_eq!(column(&result, "age"), [Value::from(25), Value::from(30)]);
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
    assert_eq!(strings(&result, "s"), ["30"]);
}

#[test]
fn query_function_to_upper() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN toUpper(n.name) AS name"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["ALICE"]);
}

#[test]
fn query_function_size() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher(r#"MATCH (n:Person) RETURN size(n.name) AS len"#)
        .unwrap();
    assert_eq!(column(&result, "len"), [Value::from(5)]);
}

#[test]
fn query_return_parameter_value() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher_params(
            r#"MATCH (n:Person) RETURN $requested_name AS name LIMIT 1"#,
            &params([("requested_name", "Alice".into())]),
        )
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice"]);
}

#[test]
fn query_function_argument_parameter() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g
        .cypher_params(
            r#"MATCH (n:Person) RETURN toUpper($name) AS upper LIMIT 1"#,
            &params([("name", "Alice".into())]),
        )
        .unwrap();
    assert_eq!(strings(&result, "upper"), ["ALICE"]);
}

#[test]
fn query_missing_parameter_returns_error() {
    let g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    let result = g.cypher_params(
        "MATCH (n:Person) WHERE n.name = $name RETURN n",
        &Parameters::new(),
    );
    assert!(
        matches!(&result, Err(CypherError::InvalidQuery(msg)) if msg.contains("name")),
        "{result:?}"
    );
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
    assert_eq!(strings(&result, "a_name"), ["Alice"]);
    assert_eq!(column(&result, "b_name"), [Value::Null]);
}

// ---------------------------------------------------------------------------
// SET / REMOVE
// ---------------------------------------------------------------------------

#[test]
fn query_mut_set_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) SET n.age = 30"#).unwrap();
    let result = g.cypher(r#"MATCH (n:Person) RETURN n.age AS age"#).unwrap();
    assert_eq!(column(&result, "age"), [Value::from(30)]);
}

#[test]
fn query_mut_with_parameters() {
    let mut g = build_graph_from_cypher(
        r#"CREATE (a:Person {name: "Alice"})
           CREATE (b:Person {name: "Bob"})"#,
    )
    .unwrap();

    g.cypher_mut_params(
        r#"MATCH (n:Person) WHERE n.name = $name SET n.seen = true"#,
        &params([("name", "Alice".into())]),
    )
    .unwrap();
    let result = g
        .cypher("MATCH (n:Person) RETURN n.name AS name, n.seen AS seen ORDER BY name")
        .unwrap();
    assert_eq!(column(&result, "seen"), [Value::from(true), Value::Null]);

    // Without the parameter the query fails instead of matching nothing.
    let err = g
        .cypher_mut(r#"MATCH (n:Person) WHERE n.name = $name SET n.seen = true"#)
        .unwrap_err();
    assert!(
        matches!(&err, CypherError::InvalidQuery(msg) if msg.contains("name")),
        "{err:?}"
    );
}

#[test]
fn query_mut_remove_property() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice", age: 30})"#).unwrap();

    g.cypher_mut(r#"MATCH (n:Person) REMOVE n.age"#).unwrap();
    let result = g.cypher(r#"MATCH (n:Person) RETURN n.age AS age"#).unwrap();
    assert_eq!(column(&result, "age"), [Value::Null]);
}

#[test]
fn query_mut_set_labels() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();
    g.cypher_mut(r#"MATCH (n:Person) SET n:Employee"#).unwrap();
    let result = g
        .cypher(r#"MATCH (n:Employee) RETURN n.name AS name"#)
        .unwrap();
    assert_eq!(strings(&result, "name"), ["Alice"]);
}

#[test]
fn query_mut_remove_labels() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person:Employee {name: "Alice"})"#).unwrap();
    g.cypher_mut(r#"MATCH (n) REMOVE n:Employee"#).unwrap();
    let result = g.cypher(r#"MATCH (n:Employee) RETURN n"#).unwrap();
    assert!(result.is_empty());
    let node = g.node_indices().next().unwrap();
    assert_eq!(g[node].labels, ["Person"]);
}

#[test]
fn query_mut_set_labels_with_custom_weights() {
    let mut g = build_graph_from_cypher_typed::<CustomNode, CustomEdge>(
        r#"CREATE (:Person {name: "Alice", age: 30})-[:KNOWS]->(:Person {name: "Bob"})"#,
    )
    .unwrap();
    g.cypher_mut(
        r#"MATCH (n {name: "Alice"})-[r:KNOWS]->() SET n:Employee, r.since = 2020 REMOVE n.age"#,
    )
    .unwrap();

    let alice = g
        .node_weights()
        .find(|n| n.property("name") == Some(&Value::from("Alice")))
        .unwrap();
    assert!(alice.has_label("Employee"));
    assert_eq!(alice.property("age"), None);
    let edge = g.edge_indices().next().unwrap();
    assert_eq!(g[edge].property("since"), Some(&Value::from(2020)));
}

// ---------------------------------------------------------------------------
// MERGE ON CREATE / ON MATCH
// ---------------------------------------------------------------------------

#[test]
fn query_mut_merge_on_create() {
    let mut g: Graph<NodeData, EdgeData> = Graph::new();

    g.cypher_mut(r#"MERGE (n:Person {name: "Alice"}) ON CREATE SET n.created = true"#)
        .unwrap();
    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.created AS created"#)
        .unwrap();
    assert_eq!(column(&result, "created"), [Value::from(true)]);
}

#[test]
fn query_mut_merge_on_match() {
    let mut g = build_graph_from_cypher(r#"CREATE (n:Person {name: "Alice"})"#).unwrap();

    g.cypher_mut(r#"MERGE (n:Person {name: "Alice"}) ON MATCH SET n.seen = true"#)
        .unwrap();
    let result = g
        .cypher(r#"MATCH (n:Person) RETURN n.seen AS seen"#)
        .unwrap();
    assert_eq!(column(&result, "seen"), [Value::from(true)]);
    assert_eq!(g.node_count(), 1);
}

// ---------------------------------------------------------------------------
// Failure semantics
// ---------------------------------------------------------------------------

#[test]
fn query_mut_failure_rolls_back_all_changes() {
    let mut g = build_graph_from_cypher("CREATE (:A {k: 1, keep: 'x'})-[:R]->(:B)").unwrap();
    let before = format!("{g:?}");
    for query in [
        "CREATE (:C) RETURN 1 / 0",
        "MATCH (a:A) SET a.k = 2, a:Extra REMOVE a.keep CREATE (a)-[:S]->(:D) RETURN 1 / 0",
        "MATCH (a:A) SET a = {k: [1, {nested: 1}]}",
        "MATCH (a:A) DETACH DELETE a WITH 1 AS one RETURN 1 / 0",
    ] {
        assert!(g.cypher_mut(query).is_err(), "{query} should fail");
        assert_eq!(
            format!("{g:?}"),
            before,
            "{query} must leave the graph unchanged"
        );
    }
}

#[test]
fn query_mut_rejects_invalid_relationship_before_creating_nodes() {
    let mut g = Graph::<NodeData, EdgeData>::new();
    assert!(
        g.cypher_mut("CREATE (a)-[:R {k: [1, {m: 1}]}]->(b)")
            .is_err()
    );
    assert_eq!((g.node_count(), g.edge_count()), (0, 0));
}

#[test]
fn query_mut_bound_node_parameter_must_exist() {
    let mut g = Graph::<NodeData, EdgeData>::new();
    let params = Parameters::from([(
        "n".to_string(),
        Value::Node(petgraph::graph::NodeIndex::new(7)),
    )]);
    let err = g
        .cypher_mut_params("WITH $n AS n CREATE (n)-[:R]->(:X)", &params)
        .unwrap_err();
    assert!(matches!(err, CypherError::RuntimeError(_)), "{err:?}");
    assert_eq!(g.node_count(), 0);
}

#[test]
fn query_non_boolean_predicate_is_a_type_error() {
    // Values only known at runtime; decypher rejects literal ones up front.
    let g = build_graph_from_cypher("CREATE (:A {k: 1})").unwrap();
    for query in [
        "MATCH (n) WHERE n.k RETURN n",
        "MATCH (n) RETURN CASE WHEN n.k THEN 1 ELSE 2 END AS x",
    ] {
        let err = g.cypher(query).unwrap_err();
        assert!(
            matches!(err, CypherError::TypeMismatch(_)),
            "{query}: {err:?}"
        );
    }
    let r = g.cypher("MATCH (n) WHERE null RETURN n").unwrap();
    assert!(r.is_empty());
}

#[test]
fn query_functions_propagate_null() {
    let g = Graph::<NodeData, EdgeData>::new();
    let r = g
        .cypher(
            "RETURN substring('abc', null) AS a, substring('abc', 1, null) AS b, \
             left('abc', null) AS c, right(null, 1) AS d, range(1, null) AS e",
        )
        .unwrap();
    assert_eq!(
        r.rows().next().unwrap().values(),
        vec![Value::Null; 5].as_slice()
    );
}

#[test]
fn query_aggregate_arity_is_checked() {
    let g = Graph::<NodeData, EdgeData>::new();
    for query in [
        "UNWIND [1, 2] AS x RETURN sum(x, 1) AS s",
        "UNWIND [1, 2] AS x RETURN percentileCont(x) AS p",
    ] {
        assert!(g.cypher(query).is_err(), "{query} should be rejected");
    }
    let r = g
        .cypher("UNWIND [1, 2, 3] AS x RETURN percentileDisc(x, 0.5) AS p")
        .unwrap();
    assert_eq!(column(&r, "p"), vec![Value::from(2)]);
}

#[test]
fn query_bound_variable_length_relationship_respects_bounds() {
    let g = build_graph_from_cypher("CREATE (:A)-[:R]->()-[:R]->()-[:R]->(:Z)").unwrap();
    let r = g
        .cypher("MATCH (:A)-[rs:R*2]->(m) WITH rs MATCH (a)-[rs*1..2]->(b) RETURN count(*) AS n")
        .unwrap();
    assert_eq!(column(&r, "n"), vec![Value::from(1)]);
    let r = g
        .cypher("MATCH (:A)-[rs:R*2]->(m) WITH rs MATCH (a)-[rs*3..4]->(b) RETURN count(*) AS n")
        .unwrap();
    assert_eq!(column(&r, "n"), vec![Value::from(0)]);
}

#[test]
fn query_null_dynamic_label_matches_nothing() {
    let g = build_graph_from_cypher("CREATE (:A)").unwrap();
    let r = g.cypher("MATCH (n:$(null)) RETURN count(*) AS n").unwrap();
    assert_eq!(column(&r, "n"), vec![Value::from(0)]);
}

// ---------------------------------------------------------------------------
// UNION branches
// ---------------------------------------------------------------------------

#[test]
fn query_union_branches_read_their_own_parameters() {
    let g = Graph::<NodeData, EdgeData>::new();
    let q = Query::new("RETURN $a AS x UNION ALL RETURN $b AS x UNION ALL RETURN $a AS x").unwrap();
    assert_eq!(q.parameters().collect::<Vec<_>>(), ["a", "b"]);

    let params = Parameters::from([
        ("a".to_string(), Value::from(1)),
        ("b".to_string(), Value::from(2)),
    ]);
    let r = q.execute(&g, &params).unwrap();
    assert_eq!(
        column(&r, "x"),
        vec![Value::from(1), Value::from(2), Value::from(1)]
    );

    let distinct = Query::new("RETURN $a AS x UNION RETURN $a AS x").unwrap();
    let r = distinct.execute(&g, &params).unwrap();
    assert_eq!(column(&r, "x"), vec![Value::from(1)]);
}

#[test]
fn query_union_missing_parameter_in_later_branch_is_an_error() {
    let g = Graph::<NodeData, EdgeData>::new();
    let q = Query::new("RETURN 1 AS x UNION RETURN $b AS x").unwrap();
    let err = q
        .execute(&g, &Parameters::from([("a".to_string(), Value::from(1))]))
        .unwrap_err();
    assert!(
        matches!(&err, CypherError::InvalidQuery(m) if m.contains('b')),
        "{err:?}"
    );
}

#[test]
fn query_union_branch_matches_in_its_own_scope() {
    let mut g = build_graph_from_cypher("CREATE (:A {v: 1}), (:B {v: 2})").unwrap();
    g.cypher_mut("MATCH (n:A) SET n.seen = true").unwrap();
    let r = g
        .cypher("MATCH (n:A) RETURN n.v AS v UNION ALL MATCH (n:B) WHERE n.v = $v RETURN n.v AS v")
        .err();
    // A missing $v fails before any branch runs.
    assert!(r.is_some());
    let r = g
        .cypher_params(
            "MATCH (n:A) RETURN n.v AS v UNION ALL MATCH (n:B) WHERE n.v = $v RETURN n.v AS v",
            &Parameters::from([("v".to_string(), Value::from(2))]),
        )
        .unwrap();
    assert_eq!(column(&r, "v"), vec![Value::from(1), Value::from(2)]);
}
