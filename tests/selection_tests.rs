//! Selections: collecting query results and running petgraph on them.

use petgraph::Graph;
use petgraph::algo::{has_path_connecting, tarjan_scc};
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::stable_graph::StableGraph;
use petgraph_decypher::{
    CypherGraph, CypherNode, EdgeData, NodeData, PetgraphCypher, PetgraphCypherMut, Selection,
    Value, build_graph_from_cypher,
};

/// The node whose `name` property is `name`.
fn node<G: CypherGraph>(g: &G, name: &str) -> NodeIndex {
    g.node_ids()
        .find(|&n| g.node(n).unwrap().property("name") == Some(&Value::from(name)))
        .unwrap_or_else(|| panic!("no node named {name}"))
}

fn names<G: CypherGraph>(g: &G, nodes: impl IntoIterator<Item = NodeIndex>) -> Vec<String> {
    let mut names: Vec<String> = nodes
        .into_iter()
        .map(|n| match g.node(n).unwrap().property("name") {
            Some(Value::String(s)) => s.clone(),
            other => panic!("unnamed node: {other:?}"),
        })
        .collect();
    names.sort();
    names
}

/// `a -[:R]-> b -[:R]-> c`, and `d` on its own.
fn chain() -> Graph<NodeData, EdgeData> {
    build_graph_from_cypher(
        "CREATE (:A {name: 'a'})-[:R]->(:B {name: 'b'})-[:R]->(:C {name: 'c'}), (:D {name: 'd'})",
    )
    .unwrap()
}

#[test]
fn selection_collects_nodes_relationships_and_paths() {
    let g = chain();
    let sel = g.cypher_select("MATCH (n:A) RETURN n").unwrap();
    assert_eq!(names(&g, sel.nodes()), ["a"]);
    assert_eq!(sel.edge_count(), 0);

    // A relationship brings its end nodes.
    let sel = g.cypher_select("MATCH ()-[r]->(:C) RETURN r").unwrap();
    assert_eq!(names(&g, sel.nodes()), ["b", "c"]);
    assert_eq!(sel.edge_count(), 1);

    let sel = g
        .cypher_select("MATCH p = (:A)-[*]->(:C) RETURN p")
        .unwrap();
    assert_eq!(names(&g, sel.nodes()), ["a", "b", "c"]);
    assert_eq!(sel.edge_count(), 2);
}

#[test]
fn selection_looks_inside_lists_and_maps() {
    let g = chain();
    let sel = g
        .cypher_select("MATCH (n) WHERE n.name <> 'd' RETURN collect(n) AS ns")
        .unwrap();
    assert_eq!(names(&g, sel.nodes()), ["a", "b", "c"]);
    let sel = g.cypher_select("MATCH (n:D) RETURN {x: [n]} AS m").unwrap();
    assert_eq!(names(&g, sel.nodes()), ["d"]);
}

#[test]
fn selection_ignores_other_values_and_empty_results() {
    let g = chain();
    let sel = g.cypher_select("RETURN 1 AS one, 'x' AS s").unwrap();
    assert!(sel.is_empty());
    let sel = g.cypher_select("MATCH (n:Missing) RETURN n").unwrap();
    assert!(sel.is_empty());
    let sub = sel.subgraph(&g);
    assert_eq!((sub.graph.node_count(), sub.graph.edge_count()), (0, 0));
}

#[test]
fn induced_view_runs_petgraph_algorithms_with_original_indices() {
    let mut g = StableGraph::<NodeData, EdgeData>::new();
    // A deleted node leaves a hole at index 0, so compacted indices would
    // differ from the original ones.
    g.cypher_mut("CREATE (:Tmp)").unwrap();
    g.cypher_mut(
        "CREATE (a:Page {name: 'a'})-[:LINKS]->(b:Page {name: 'b'})-[:LINKS]->(a), \
         (b)-[:LINKS]->(:Ad {name: 'ad'})-[:LINKS]->(b)",
    )
    .unwrap();
    g.cypher_mut("MATCH (t:Tmp) DELETE t").unwrap();

    // The whole graph is one component; the view leaves the ad out. Looking
    // the names up in `g` shows the view reports original indices.
    assert_eq!(names(&g, tarjan_scc(&g).concat()), ["a", "ad", "b"]);
    let pages = g.cypher_select("MATCH (p:Page) RETURN p").unwrap();
    let components = tarjan_scc(&pages.induced(&g));
    assert_eq!(components.len(), 1);
    assert_eq!(names(&g, components[0].iter().copied()), ["a", "b"]);
}

#[test]
fn edge_filtered_view_follows_only_selected_edges() {
    let g = chain();
    let (a, c) = (node(&g, "a"), node(&g, "c"));
    let first = g.cypher_select("MATCH (:A)-[r]->() RETURN r").unwrap();
    assert!(!has_path_connecting(&first.edge_filtered(&g), a, c, None));
    let both = g.cypher_select("MATCH ()-[r:R]->() RETURN r").unwrap();
    assert!(has_path_connecting(&both.edge_filtered(&g), a, c, None));
}

#[test]
fn subgraph_copies_the_selection_and_maps_back() {
    let mut g = StableGraph::<NodeData, EdgeData>::new();
    g.cypher_mut("CREATE (:Tmp)").unwrap();
    g.cypher_mut(
        "CREATE (:A {name: 'a'})-[:R {w: 1}]->(:B {name: 'b'})-[:R {w: 2}]->(:C {name: 'c'}), \
         (:D {name: 'd'})",
    )
    .unwrap();
    g.cypher_mut("MATCH (t:Tmp) DELETE t").unwrap();

    let sel: Selection = g
        .cypher_select("MATCH p = (:A)-[*]->(:C) RETURN p")
        .unwrap();
    let sub = sel.subgraph(&g);
    assert_eq!((sub.graph.node_count(), sub.graph.edge_count()), (3, 2));
    for n in sub.graph.node_indices() {
        let original = sub.original_node(n).unwrap();
        assert_eq!(sub.graph[n], g[original]);
    }
    for e in sub.graph.edge_indices() {
        let original = sub.original_edge(e).unwrap();
        assert_eq!(sub.graph[e], g[original]);
        let (s, t) = sub.graph.edge_endpoints(e).unwrap();
        let mapped = (sub.original_node(s).unwrap(), sub.original_node(t).unwrap());
        assert_eq!(Some(mapped), g.edge_endpoints(original));
    }
    assert_eq!(sub.original_node(NodeIndex::new(3)), None);
    assert_eq!(sub.original_edge(EdgeIndex::new(2)), None);
}
