# petgraph-cypher

Build [`petgraph`](https://crates.io/crates/petgraph) graphs from
[OpenCypher](https://opencypher.org/) queries.

## Overview

`petgraph-cypher` provides a function that parses a Cypher query string and
materialises the `CREATE` / `MERGE` operations into a `petgraph::Graph`.

```rust
use petgraph_cypher::build_graph_from_cypher;

let graph = build_graph_from_cypher(
    r#"CREATE (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#,
)
.unwrap();

assert_eq!(graph.node_count(), 2);
assert_eq!(graph.edge_count(), 1);
```

## Supported Cypher subset

| Feature | Status |
|---------|--------|
| `CREATE (n:Label {k: v})-[:TYPE]->(m)` | ✅ materialised + executed |
| `MERGE  (n:Label {k: v})-[:TYPE]->(m)` | ✅ materialised + executed |
| `MATCH  (n)-[r]->(m) WHERE n.p = v`    | ✅ evaluated |
| `RETURN n, n.prop AS alias` / `RETURN *` | ✅ evaluated |
| `[DETACH] DELETE n`                    | ✅ executed |
| Multiple clauses in one query          | ✅ |
| Semicolon-separated statements         | ✅ |
| Multi-label nodes `(n:A:B)`            | ✅ |
| Undirected relationships `-[r]-`       | ✅ |
| String, integer, float, bool, null     | ✅ |

## API

```rust
// Parse only – returns the HIR-backed query plan
let query = petgraph_cypher::parse_cypher("CREATE (n:Person {name: \"Alice\"})")
    .unwrap();

// Parse + build graph – executes CREATE/MERGE clauses
let graph = petgraph_cypher::build_graph_from_cypher(
    "CREATE (a)-[:KNOWS]->(b)"
)
.unwrap();

// Read-only query execution via PetgraphCypher trait
use petgraph_cypher::PetgraphCypher;
let result = graph.cypher("MATCH (n) RETURN n").unwrap();

// Mutating query execution via PetgraphCypher trait
graph.cypher_mut("CREATE (n:Person {name: \"Alice\"})").unwrap();
```

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

Licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
