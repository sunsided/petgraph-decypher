//! Converts a parsed [`CypherQuery`] AST into petgraph graph operations.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use petgraph::Graph;

use crate::ast::*;
use crate::error::CypherError;
use crate::{EdgeData, NodeData};

/// Build a petgraph [`Graph`] by executing all `CREATE` and `MERGE` clauses
/// found in the parsed query.
///
/// Variables that appear in multiple patterns refer to the same node in the
/// resulting graph.
pub(crate) fn build_graph(query: CypherQuery) -> Result<Graph<NodeData, EdgeData>, CypherError> {
    let mut graph: Graph<NodeData, EdgeData> = Graph::new();
    // Tracks variable name → NodeIndex for reuse across patterns.
    let mut var_map: HashMap<String, NodeIndex> = HashMap::new();

    for clause in query.clauses {
        match clause {
            Clause::Create { patterns } => {
                for pattern in patterns {
                    apply_path_pattern(&mut graph, &mut var_map, &pattern);
                }
            }
            Clause::Merge { pattern } => {
                apply_path_pattern(&mut graph, &mut var_map, &pattern);
            }
            // MATCH, RETURN, DELETE do not create nodes/edges when building a
            // new graph; they are included in the AST but ignored here.
            _ => {}
        }
    }

    Ok(graph)
}

/// Walk a single path pattern, adding nodes and edges to the graph.
fn apply_path_pattern(
    graph: &mut Graph<NodeData, EdgeData>,
    var_map: &mut HashMap<String, NodeIndex>,
    pattern: &PathPattern,
) {
    let mut prev_idx = get_or_add_node(graph, var_map, &pattern.start);

    for (rel, target_node) in &pattern.rels {
        let target_idx = get_or_add_node(graph, var_map, target_node);

        let edge_data = EdgeData {
            variable: rel.variable.clone(),
            rel_type: rel.rel_type.clone(),
            properties: rel.properties.clone(),
        };

        match rel.direction {
            RelDirection::Right => {
                graph.add_edge(prev_idx, target_idx, edge_data);
            }
            RelDirection::Left => {
                graph.add_edge(target_idx, prev_idx, edge_data);
            }
            RelDirection::Both => {
                // Undirected relationships are stored as a single directed edge
                // from source to target. Direction semantics must be preserved
                // at the query/AST level.
                graph.add_edge(prev_idx, target_idx, edge_data);
            }
        }

        prev_idx = target_idx;
    }
}

/// Return the `NodeIndex` for a node pattern.  If the pattern has a variable
/// that was already added, the existing index is returned so that later
/// patterns can reference the same node.  Otherwise a new node is inserted.
fn get_or_add_node(
    graph: &mut Graph<NodeData, EdgeData>,
    var_map: &mut HashMap<String, NodeIndex>,
    pattern: &NodePattern,
) -> NodeIndex {
    if let Some(var) = &pattern.variable {
        if let Some(&idx) = var_map.get(var) {
            return idx;
        }
    }

    let data = NodeData {
        variable: pattern.variable.clone(),
        labels: pattern.labels.clone(),
        properties: pattern.properties.clone(),
    };

    let idx = graph.add_node(data);

    if let Some(var) = &pattern.variable {
        var_map.insert(var.clone(), idx);
    }

    idx
}
