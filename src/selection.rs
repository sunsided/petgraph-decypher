//! Graph slices selected with Cypher, as petgraph views or owned subgraphs.

use std::collections::BTreeSet;

use petgraph::Graph;
use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::{EdgeFiltered, EdgeRef, GraphBase, IntoEdgeReferences, NodeFiltered};

use crate::graph::CypherGraph;
use crate::result::QueryResult;
use crate::value::Value;

/// The nodes and relationships a query returned, by their index in the
/// queried graph.
///
/// Run petgraph algorithms on the selected slice through [`induced`] or
/// [`edge_filtered`], which keep the original indices, or copy it out with
/// [`subgraph`].
///
/// ```
/// use petgraph::algo::tarjan_scc;
/// use petgraph_decypher::{PetgraphCypher, build_graph_from_cypher};
///
/// let graph = build_graph_from_cypher(
///     "CREATE (a:Page)-[:LINKS]->(b:Page)-[:LINKS]->(a), (b)-[:LINKS]->(:Ad)",
/// )?;
/// let pages = graph.cypher_select("MATCH (p:Page) RETURN p")?;
/// let components = tarjan_scc(&pages.induced(&graph));
/// assert_eq!(components.len(), 1); // the two pages, without the ad
/// assert_eq!(components[0].len(), 2);
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
///
/// [`induced`]: Selection::induced
/// [`edge_filtered`]: Selection::edge_filtered
/// [`subgraph`]: Selection::subgraph
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    nodes: BTreeSet<NodeIndex>,
    edges: BTreeSet<EdgeIndex>,
}

impl Selection {
    /// Collects every node, relationship and path in `result`, including
    /// those inside lists and maps. A relationship also selects both of its
    /// end nodes, which are looked up in `graph`.
    ///
    /// `graph` must be the graph `result` was computed on, unchanged since.
    /// A selection holds plain indices, which cannot be checked against a
    /// graph: with another graph, or after a `DELETE` on a
    /// [`petgraph::Graph`] (which moves elements to fill the gap), they
    /// silently refer to other elements.
    pub fn from_result<G: CypherGraph>(result: &QueryResult, graph: &G) -> Self {
        let mut selection = Self::default();
        for row in result.rows() {
            for value in row.values() {
                selection.add(value, graph);
            }
        }
        selection
    }

    fn add<G: CypherGraph>(&mut self, value: &Value, graph: &G) {
        match value {
            Value::Node(n) => {
                self.nodes.insert(*n);
            }
            Value::Relationship(e) => {
                self.edges.insert(*e);
                if let Some((source, target)) = graph.endpoints(*e) {
                    self.nodes.extend([source, target]);
                }
            }
            Value::Path(p) => {
                self.nodes.extend(&p.nodes);
                self.edges.extend(&p.relationships);
            }
            Value::List(items) => items.iter().for_each(|v| self.add(v, graph)),
            Value::Map(map) => map.values().for_each(|v| self.add(v, graph)),
            _ => {}
        }
    }

    /// The selected nodes, in ascending index order.
    pub fn nodes(&self) -> impl ExactSizeIterator<Item = NodeIndex> + '_ {
        self.nodes.iter().copied()
    }

    /// The selected relationships, in ascending index order.
    pub fn edges(&self) -> impl ExactSizeIterator<Item = EdgeIndex> + '_ {
        self.edges.iter().copied()
    }

    /// Whether `node` is selected.
    pub fn contains_node(&self, node: NodeIndex) -> bool {
        self.nodes.contains(&node)
    }

    /// Whether `edge` is selected.
    pub fn contains_edge(&self, edge: EdgeIndex) -> bool {
        self.edges.contains(&edge)
    }

    /// The number of selected nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The number of selected relationships.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Whether nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }

    /// A view of `graph` with only the selected nodes and every edge between
    /// them. Indices stay those of `graph`, so petgraph algorithms run on the
    /// view and their results refer to the original graph.
    pub fn induced<G>(&self, graph: G) -> NodeFiltered<G, impl Fn(NodeIndex) -> bool + '_>
    where
        G: GraphBase<NodeId = NodeIndex>,
    {
        NodeFiltered(graph, move |n: NodeIndex| self.nodes.contains(&n))
    }

    /// A view of `graph` with only the selected edges. Nodes are not
    /// filtered: unselected nodes stay in the view, without the edges that
    /// were left out. Indices stay those of `graph`.
    pub fn edge_filtered<G>(&self, graph: G) -> EdgeFiltered<G, impl Fn(G::EdgeRef) -> bool + '_>
    where
        G: IntoEdgeReferences<EdgeId = EdgeIndex>,
    {
        EdgeFiltered(graph, move |e: G::EdgeRef| self.edges.contains(&e.id()))
    }

    /// Copies the selected nodes and relationships into a new graph. Nodes
    /// keep their order, and so do relationships.
    ///
    /// Only *selected* relationships are copied: a selection of nodes alone
    /// yields isolated nodes, while [`Selection::induced`] also shows the
    /// edges between them. For an owned induced copy, select the
    /// relationships in the query too, or use petgraph's `filter_map` with
    /// [`Selection::contains_node`].
    ///
    /// Elements `graph` does not have, and relationships whose end nodes are
    /// not both copied, are left out; with the graph the selection came from
    /// (see [`Selection::from_result`]) that does not happen.
    pub fn subgraph<G>(&self, graph: &G) -> Subgraph<G::Node, G::Edge>
    where
        G: CypherGraph,
        G::Node: Clone,
        G::Edge: Clone,
    {
        let mut out = Subgraph {
            graph: Graph::new(),
            nodes: Vec::with_capacity(self.nodes.len()),
            edges: Vec::with_capacity(self.edges.len()),
        };
        // New node indices are dense, so the position in `out.nodes` is the
        // new index.
        for &n in &self.nodes {
            if let Some(weight) = graph.node(n) {
                out.graph.add_node(weight.clone());
                out.nodes.push(n);
            }
        }
        let new_index = |n: NodeIndex| out.nodes.binary_search(&n).ok().map(NodeIndex::new);
        let mut edges = Vec::with_capacity(self.edges.len());
        for &e in &self.edges {
            let (Some((s, t)), Some(weight)) = (graph.endpoints(e), graph.edge(e)) else {
                continue;
            };
            if let (Some(s), Some(t)) = (new_index(s), new_index(t)) {
                edges.push((s, t, weight.clone(), e));
            }
        }
        for (s, t, weight, e) in edges {
            out.graph.add_edge(s, t, weight);
            out.edges.push(e);
        }
        out
    }
}

/// A selection copied into a graph of its own, with the way back to the
/// original indices.
///
/// The fields are plain data: adding or removing elements of `graph`, or
/// changing `nodes` / `edges`, breaks the mapping that
/// [`Subgraph::original_node`] and [`Subgraph::original_edge`] read.
#[derive(Debug, Clone)]
pub struct Subgraph<N, E> {
    /// The copied nodes and relationships.
    pub graph: Graph<N, E>,
    /// `nodes[i]` is the original index of node `i` of `graph`.
    pub nodes: Vec<NodeIndex>,
    /// `edges[i]` is the original index of edge `i` of `graph`.
    pub edges: Vec<EdgeIndex>,
}

impl<N, E> Subgraph<N, E> {
    /// The original index of `node`, a node of [`Subgraph::graph`].
    pub fn original_node(&self, node: NodeIndex) -> Option<NodeIndex> {
        self.nodes.get(node.index()).copied()
    }

    /// The original index of `edge`, an edge of [`Subgraph::graph`].
    pub fn original_edge(&self, edge: EdgeIndex) -> Option<EdgeIndex> {
        self.edges.get(edge.index()).copied()
    }
}
