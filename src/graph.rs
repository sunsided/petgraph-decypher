//! The graph operations the engine needs, implemented for petgraph's
//! [`Graph`] and [`StableGraph`] (directed, default index type).

use petgraph::Direction;
use petgraph::graph::{EdgeIndex, Graph, NodeIndex};
use petgraph::stable_graph::StableGraph;
use petgraph::visit::EdgeRef;

use crate::element::{CypherEdge, CypherEdgeMut, CypherNode, CypherNodeMut};

/// Read access to a graph Cypher queries run against.
pub trait CypherGraph {
    /// The node weight.
    type Node: CypherNode;
    /// The relationship (edge) weight.
    type Edge: CypherEdge;

    /// All node indices.
    fn node_ids(&self) -> impl Iterator<Item = NodeIndex>;
    /// All edge indices.
    fn edge_ids(&self) -> impl Iterator<Item = EdgeIndex>;
    /// The weight of node `id`.
    fn node(&self, id: NodeIndex) -> Option<&Self::Node>;
    /// The weight of edge `id`.
    fn edge(&self, id: EdgeIndex) -> Option<&Self::Edge>;
    /// The `(source, target)` of edge `id`.
    fn endpoints(&self, id: EdgeIndex) -> Option<(NodeIndex, NodeIndex)>;
    /// The edges leaving (`Outgoing`) or entering (`Incoming`) `node`, each
    /// with the node at its other end.
    fn edges_of(
        &self,
        node: NodeIndex,
        direction: Direction,
    ) -> impl Iterator<Item = (EdgeIndex, NodeIndex)>;
}

/// Write access for mutating queries.
pub trait CypherGraphMut: CypherGraph<Node: CypherNodeMut, Edge: CypherEdgeMut> {
    /// Mutable weight of node `id`.
    fn node_mut(&mut self, id: NodeIndex) -> Option<&mut Self::Node>;
    /// Mutable weight of edge `id`.
    fn edge_mut(&mut self, id: EdgeIndex) -> Option<&mut Self::Edge>;
    /// Adds a node.
    fn add_node(&mut self, weight: Self::Node) -> NodeIndex;
    /// Adds an edge from `source` to `target`.
    fn add_edge(&mut self, source: NodeIndex, target: NodeIndex, weight: Self::Edge) -> EdgeIndex;
    /// Removes the given edges and nodes. Callers guarantee that every edge
    /// of a removed node is among `edges`.
    fn remove(&mut self, nodes: Vec<NodeIndex>, edges: Vec<EdgeIndex>);
}

fn other_end(node: NodeIndex, source: NodeIndex, target: NodeIndex) -> NodeIndex {
    if source == node { target } else { source }
}

impl<N: CypherNode, E: CypherEdge> CypherGraph for Graph<N, E> {
    type Node = N;
    type Edge = E;

    fn node_ids(&self) -> impl Iterator<Item = NodeIndex> {
        self.node_indices()
    }

    fn edge_ids(&self) -> impl Iterator<Item = EdgeIndex> {
        self.edge_indices()
    }

    fn node(&self, id: NodeIndex) -> Option<&N> {
        self.node_weight(id)
    }

    fn edge(&self, id: EdgeIndex) -> Option<&E> {
        self.edge_weight(id)
    }

    fn endpoints(&self, id: EdgeIndex) -> Option<(NodeIndex, NodeIndex)> {
        self.edge_endpoints(id)
    }

    fn edges_of(
        &self,
        node: NodeIndex,
        direction: Direction,
    ) -> impl Iterator<Item = (EdgeIndex, NodeIndex)> {
        self.edges_directed(node, direction)
            .map(move |e| (e.id(), other_end(node, e.source(), e.target())))
    }
}

impl<N: CypherNodeMut, E: CypherEdgeMut> CypherGraphMut for Graph<N, E> {
    fn node_mut(&mut self, id: NodeIndex) -> Option<&mut N> {
        self.node_weight_mut(id)
    }

    fn edge_mut(&mut self, id: EdgeIndex) -> Option<&mut E> {
        self.edge_weight_mut(id)
    }

    fn add_node(&mut self, weight: N) -> NodeIndex {
        Graph::add_node(self, weight)
    }

    fn add_edge(&mut self, source: NodeIndex, target: NodeIndex, weight: E) -> EdgeIndex {
        Graph::add_edge(self, source, target, weight)
    }

    /// `Graph` removal moves the last element into the freed slot, so remove
    /// in descending index order: the moved element then always has a higher
    /// index than any element still to be removed.
    fn remove(&mut self, mut nodes: Vec<NodeIndex>, mut edges: Vec<EdgeIndex>) {
        edges.sort_unstable_by(|a, b| b.cmp(a));
        edges.dedup();
        for edge in edges {
            self.remove_edge(edge);
        }
        nodes.sort_unstable_by(|a, b| b.cmp(a));
        nodes.dedup();
        for node in nodes {
            self.remove_node(node);
        }
    }
}

impl<N: CypherNode, E: CypherEdge> CypherGraph for StableGraph<N, E> {
    type Node = N;
    type Edge = E;

    fn node_ids(&self) -> impl Iterator<Item = NodeIndex> {
        self.node_indices()
    }

    fn edge_ids(&self) -> impl Iterator<Item = EdgeIndex> {
        self.edge_indices()
    }

    fn node(&self, id: NodeIndex) -> Option<&N> {
        self.node_weight(id)
    }

    fn edge(&self, id: EdgeIndex) -> Option<&E> {
        self.edge_weight(id)
    }

    fn endpoints(&self, id: EdgeIndex) -> Option<(NodeIndex, NodeIndex)> {
        self.edge_endpoints(id)
    }

    fn edges_of(
        &self,
        node: NodeIndex,
        direction: Direction,
    ) -> impl Iterator<Item = (EdgeIndex, NodeIndex)> {
        self.edges_directed(node, direction)
            .map(move |e| (e.id(), other_end(node, e.source(), e.target())))
    }
}

impl<N: CypherNodeMut, E: CypherEdgeMut> CypherGraphMut for StableGraph<N, E> {
    fn node_mut(&mut self, id: NodeIndex) -> Option<&mut N> {
        self.node_weight_mut(id)
    }

    fn edge_mut(&mut self, id: EdgeIndex) -> Option<&mut E> {
        self.edge_weight_mut(id)
    }

    fn add_node(&mut self, weight: N) -> NodeIndex {
        StableGraph::add_node(self, weight)
    }

    fn add_edge(&mut self, source: NodeIndex, target: NodeIndex, weight: E) -> EdgeIndex {
        StableGraph::add_edge(self, source, target, weight)
    }

    fn remove(&mut self, nodes: Vec<NodeIndex>, edges: Vec<EdgeIndex>) {
        for edge in edges {
            self.remove_edge(edge);
        }
        for node in nodes {
            self.remove_node(node);
        }
    }
}
