# petgraph-decypher

Cypher front-end for [`petgraph`](https://crates.io/crates/petgraph):
build, query, and mutate graphs with
[openCypher](https://opencypher.org/)-compatible queries.

<div align="center">
  <img src="https://raw.githubusercontent.com/sunsided/petgraph-decypher/refs/heads/main/.readme/banner.png" alt="petgraph-decypher crate hero picture" />
</div>

This project is independent and is not affiliated with, endorsed by, or sponsored by Neo4j, Inc.
Cypher® and Neo4j® are registered trademarks of Neo4j, Inc.

## Overview

`petgraph-decypher` is both:

1. A Cypher-driven graph builder (`CREATE` / `MERGE`), and
2. A query engine for executing read and mutation queries on `petgraph::Graph`.

You can start from Cypher **or** from an existing graph and run Cypher against it.

```rust
use petgraph_decypher::build_graph_from_cypher;

let graph = build_graph_from_cypher(
    r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
)
.unwrap();

assert_eq!(graph.node_count(), 2);
assert_eq!(graph.edge_count(), 1);
```

## What works today

| Feature | Status |
|---------|--------|
| Build graphs with `CREATE` / `MERGE` | ✅ |
| Read queries: `MATCH`, `WHERE`, `RETURN`, `OPTIONAL MATCH` | ✅ |
| Projection features: `DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT` | ✅ |
| Mutation queries: `CREATE`, `MERGE`, `SET`, `REMOVE`, `[DETACH] DELETE` | ✅ |
| Scalar functions (e.g. `size`, `toString`, `toUpper`, `toLower`, `trim`) | ✅ |
| Multiple clauses / semicolon-separated statements | ✅ |
| Multi-label nodes and undirected relationships | ✅ |
| String, integer, float, bool, null, list, map values | ✅ |

## Current limitations and planned work

| Capability | Status |
|------------|--------|
| Parameterized queries (`$param`) | 🚧 planned |
| Variable-length paths (`*1..3`) | 🚧 planned |
| Native subgraph selection as a first-class query output | 🚧 planned |
| End-to-end Cypher subgraph -> petgraph algorithm -> Cypher-style projection helpers | 🚧 planned |

## API

```rust
// Parse only – returns the HIR-backed query plan
let query = petgraph_decypher::parse_cypher("CREATE (n:Person {name: \"Alice\"})")
    .unwrap();

// Parse + build graph – executes CREATE/MERGE clauses
let graph = petgraph_decypher::build_graph_from_cypher(
    "CREATE (a)-[:KNOWS]->(b)"
)
.unwrap();

// Read-only query execution via PetgraphCypher trait
use petgraph_decypher::PetgraphCypher;
let result = graph.cypher("MATCH (n) RETURN n").unwrap();

// Mutating query execution via PetgraphCypher trait
graph.cypher_mut("CREATE (n:Person {name: \"Alice\"})").unwrap();
```

### Query an existing graph

```rust
use petgraph::Graph;
use petgraph_decypher::{CypherValue, EdgeData, NodeData, PetgraphCypher};

let mut graph = Graph::<NodeData, EdgeData>::new();
let a = graph.add_node(NodeData {
    variable: None,
    labels: vec!["Person".into()],
    properties: std::collections::HashMap::from([(
        "name".into(),
        CypherValue::String("Alice".into()),
    )]),
});
let b = graph.add_node(NodeData {
    variable: None,
    labels: vec!["Person".into()],
    properties: std::collections::HashMap::from([(
        "name".into(),
        CypherValue::String("Bob".into()),
    )]),
});
graph.add_edge(
    a,
    b,
    EdgeData {
        variable: None,
        rel_type: Some("KNOWS".into()),
        properties: Default::default(),
    },
);

let rows: Vec<_> = graph
    .cypher("MATCH (p:Person)-[:KNOWS]->(q:Person) RETURN p.name AS from, q.name AS to")
    .unwrap()
    .collect();
assert_eq!(rows.len(), 1);
```

### Mutate an existing graph

```rust
use petgraph_decypher::{PetgraphCypher, build_graph_from_cypher};

let mut graph = build_graph_from_cypher(r#"CREATE (p:Person {name: "Alice"})"#).unwrap();

graph.cypher_mut(r#"MATCH (p:Person {name: "Alice"}) SET p.role = "Archaeologist""#).unwrap();
graph.cypher_mut(r#"MATCH (p:Person {name: "Alice"}) REMOVE p.role"#).unwrap();
```

### Custom node and edge types

`build_graph_from_cypher_typed` supports domain-specific node and edge weights via
`CypherNode` and `CypherEdge`.

For a complete working reference, see:

- `tests/builder_tests.rs` (`build_graph_typed_with_custom_types`)
- `tests/query_tests.rs` (`query_match_all_nodes_with_custom_weights`)

### Showcase direction: Cypher + petgraph algorithms

Today, you can already compose Cypher selection with petgraph algorithms in user code:

1. Build/load the graph.
2. Use Cypher to select the slice of interest (e.g. nodes/relationships by label/property).
3. Run a petgraph algorithm on that selection.
4. Map algorithm output back to original node properties.

The `examples/indiana_jones.rs` example demonstrates the first two steps with read + mutation queries.

### Node data

Each petgraph node carries a `NodeData` value:

```rust
pub struct NodeData {
    pub variable:   Option<String>,
    pub labels:     Vec<String>,
    pub properties: HashMap<String, CypherValue>,
}
```

`NodeData` implements `CypherNode`, and its property map is exposed through
`CypherProperties::get`.

### Edge data

Each petgraph edge carries an `EdgeData` value:

```rust
pub struct EdgeData {
    pub variable:   Option<String>,
    pub rel_type:   Option<String>,
    pub properties: HashMap<String, CypherValue>,
}
```

`EdgeData` implements `CypherEdge`, and exposes relationship properties through
`CypherProperties::get`.

## License

Licensed under either of [EUPL-1.2](LICENSE-EUPL), [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
