//! `to_cypher_create`: writing a graph as Cypher and reading it back.

use petgraph::Direction;
use petgraph::Graph;
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::stable_graph::StableGraph;
use petgraph_decypher::{
    CypherError, CypherGraph, EdgeData, NodeData, PetgraphCypherMut, Properties, Value,
    build_graph_from_cypher, to_cypher_create,
};

fn props<const N: usize>(entries: [(&str, Value); N]) -> Properties {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
}

fn node(labels: &[&str], properties: Properties) -> NodeData {
    NodeData {
        variable: None,
        labels: labels.iter().map(|l| l.to_string()).collect(),
        properties,
    }
}

fn edge(rel_type: &str, properties: Properties) -> EdgeData {
    EdgeData {
        variable: None,
        rel_type: rel_type.to_owned(),
        properties,
    }
}

/// Property maps equal under Cypher equivalence (so `NaN` matches `NaN`).
fn same_properties(a: &Properties, b: &Properties) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((ka, va), (kb, vb))| ka == kb && va.equivalent(vb))
}

/// Asserts that `copy` has the same nodes and edges as `original`, in order.
fn assert_same<'a>(
    original: impl Iterator<Item = (&'a NodeData, Option<&'a NodeData>)>,
    edges: impl Iterator<Item = ((usize, usize, &'a EdgeData), (usize, usize, &'a EdgeData))>,
) {
    for (a, b) in original {
        let b = b.expect("node missing from the copy");
        assert_eq!(a.labels, b.labels);
        assert!(
            same_properties(&a.properties, &b.properties),
            "{:?} != {:?}",
            a.properties,
            b.properties
        );
    }
    for ((sa, ta, a), (sb, tb, b)) in edges {
        assert_eq!((sa, ta, &a.rel_type), (sb, tb, &b.rel_type));
        assert!(same_properties(&a.properties, &b.properties));
    }
}

#[test]
fn export_round_trips_awkward_names_and_values() {
    let mut g = Graph::<NodeData, EdgeData>::new();
    let a = g.add_node(node(
        &["Person", "MATCH", "with space", "back`tick"],
        props([
            (
                "name",
                Value::from("O'Brien \\ \"quoted\"\nline\r\ttab \u{1}\u{7f} é😀"),
            ),
            ("key with space", Value::from(1)),
            ("`odd`", Value::from(true)),
            ("min", Value::Integer(i64::MIN)),
            ("max", Value::Integer(i64::MAX)),
            ("tiny", Value::from(1e-7)),
            ("neg_zero", Value::from(-0.0)),
            ("nan", Value::from(f64::NAN)),
            ("inf", Value::from(f64::INFINITY)),
            ("ninf", Value::from(f64::NEG_INFINITY)),
            ("tags", Value::from(vec!["a", "b"])),
            ("nums", Value::List(vec![Value::from(1), Value::from(2.5)])),
        ]),
    ));
    let b = g.add_node(node(&[], Properties::new()));
    g.add_edge(a, b, edge("KNOWS", props([("since", Value::from(2020))])));
    g.add_edge(a, b, edge("KNOWS", Properties::new())); // parallel
    g.add_edge(b, b, edge("SELF LOOP", Properties::new()));
    g.add_edge(b, a, edge("RETURN", props([("w", Value::from(0.5))])));

    let script = to_cypher_create(&g).unwrap();
    let copy = build_graph_from_cypher(&script)
        .unwrap_or_else(|e| panic!("cannot read back:\n{script}\n{e}"));

    assert_eq!(
        (copy.node_count(), copy.edge_count()),
        (g.node_count(), g.edge_count())
    );
    let endpoints = |graph: &Graph<NodeData, EdgeData>| -> Vec<(usize, usize)> {
        graph
            .edge_indices()
            .map(|e| {
                let (s, t) = graph.edge_endpoints(e).unwrap();
                (s.index(), t.index())
            })
            .collect()
    };
    let (ea, eb) = (endpoints(&g), endpoints(&copy));
    assert_same(
        g.node_indices().map(|n| (&g[n], copy.node_weight(n))),
        g.edge_indices().zip(copy.edge_indices()).map(|(x, y)| {
            (
                (ea[x.index()].0, ea[x.index()].1, &g[x]),
                (eb[y.index()].0, eb[y.index()].1, &copy[y]),
            )
        }),
    );
}

#[test]
fn export_skips_stable_graph_holes() {
    let mut g = StableGraph::<NodeData, EdgeData>::new();
    g.cypher_mut("CREATE (:Tmp)").unwrap();
    g.cypher_mut("CREATE (:A {v: 1})-[:R]->(:B {v: 2})")
        .unwrap();
    g.cypher_mut("MATCH (t:Tmp) DELETE t").unwrap();

    let script = to_cypher_create(&g).unwrap();
    assert!(script.starts_with("CREATE (n0:A {v: 1})\n"), "{script}");
    let copy = build_graph_from_cypher(&script).unwrap();
    assert_eq!((copy.node_count(), copy.edge_count()), (2, 1));
    let labels: Vec<_> = copy.node_weights().map(|n| n.labels.clone()).collect();
    assert_eq!(labels, [["A"], ["B"]]);
    let e = copy.edge_indices().next().unwrap();
    let (s, t) = copy.edge_endpoints(e).unwrap();
    assert_eq!((s.index(), t.index()), (0, 1));
}

#[test]
fn export_of_an_empty_graph_is_empty() {
    let g = Graph::<NodeData, EdgeData>::new();
    assert_eq!(to_cypher_create(&g).unwrap(), "");
}

#[test]
fn export_rejects_values_create_cannot_store() {
    for value in [
        Value::Node(NodeIndex::new(0)),
        Value::Map(props([("k", Value::from(1))])),
        Value::List(vec![Value::from(vec![1])]),
        Value::List(vec![Value::from(1), Value::from("a")]),
    ] {
        let mut g = Graph::<NodeData, EdgeData>::new();
        g.add_node(node(&[], props([("p", value.clone())])));
        assert!(
            matches!(to_cypher_create(&g), Err(CypherError::TypeMismatch(_))),
            "{value:?} should be rejected"
        );
    }
}

#[test]
fn export_leaves_out_null_properties() {
    let mut g = Graph::<NodeData, EdgeData>::new();
    g.add_node(node(
        &["A"],
        props([("gone", Value::Null), ("kept", Value::from(1))]),
    ));
    assert_eq!(to_cypher_create(&g).unwrap(), "CREATE (n0:A {kept: 1})\n");
}

#[test]
fn export_round_trips_names_cypher_reads_verbatim() {
    // Text between backticks has no escapes, so these need nothing but
    // backtick doubling to read back unchanged.
    let names = [
        "",
        "C:\\users",
        "a\nb",
        "tab\there",
        "nul\u{0}",
        "\\u0041",
        "`",
    ];
    let mut g = Graph::<NodeData, EdgeData>::new();
    for name in names {
        let n = g.add_node(node(&[name], props([(name, Value::from(1))])));
        g.add_edge(n, n, edge(name, Properties::new()));
    }
    let script = to_cypher_create(&g).unwrap();
    let copy = build_graph_from_cypher(&script)
        .unwrap_or_else(|e| panic!("cannot read back:\n{script}\n{e}"));
    for (n, name) in copy.node_indices().zip(names) {
        assert_eq!(copy[n].labels, [name]);
        assert_eq!(copy[n].properties.keys().collect::<Vec<_>>(), [name]);
    }
    for (e, name) in copy.edge_indices().zip(names) {
        assert_eq!(copy[e].rel_type, name);
    }
}

/// A `Graph` whose node listing names one node more than it has.
struct Broken(Graph<NodeData, EdgeData>);

impl CypherGraph for Broken {
    type Node = NodeData;
    type Edge = EdgeData;

    fn node_ids(&self) -> impl Iterator<Item = NodeIndex> {
        self.0
            .node_indices()
            .chain([NodeIndex::new(self.0.node_count())])
    }
    fn edge_ids(&self) -> impl Iterator<Item = EdgeIndex> {
        self.0.edge_indices()
    }
    fn node(&self, id: NodeIndex) -> Option<&NodeData> {
        self.0.node_weight(id)
    }
    fn edge(&self, id: EdgeIndex) -> Option<&EdgeData> {
        self.0.edge_weight(id)
    }
    fn endpoints(&self, id: EdgeIndex) -> Option<(NodeIndex, NodeIndex)> {
        self.0.edge_endpoints(id)
    }
    fn edges_of(
        &self,
        node: NodeIndex,
        direction: Direction,
    ) -> impl Iterator<Item = (EdgeIndex, NodeIndex)> {
        CypherGraph::edges_of(&self.0, node, direction)
    }
}

#[test]
fn export_fails_on_elements_a_graph_lists_but_lacks() {
    let g = Broken(build_graph_from_cypher("CREATE (:A)").unwrap());
    assert!(matches!(
        to_cypher_create(&g),
        Err(CypherError::RuntimeError(_))
    ));
}
