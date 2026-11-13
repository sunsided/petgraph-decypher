//! Writing a graph back out as Cypher.

use std::collections::HashMap;
use std::fmt::Write;

use crate::element::{CypherEdge, CypherNode};
use crate::error::CypherError;
use crate::graph::CypherGraph;
use crate::value::Value;

/// Renders `graph` as a Cypher script of `CREATE` clauses, one per node and
/// then one per relationship, that [`build_graph_from_cypher`] reads back.
///
/// Nodes and relationships keep their order, so reading the script into a
/// new [`petgraph::Graph`] numbers them `0..n`. An empty graph renders as an
/// empty string. Fails if a property holds a node, relationship or path,
/// which have no literal form.
///
/// ```
/// use petgraph_decypher::{build_graph_from_cypher, to_cypher_create};
///
/// let graph = build_graph_from_cypher("CREATE (:Person {name: 'Alice'})-[:KNOWS]->(:Person)")?;
/// let script = to_cypher_create(&graph)?;
/// assert_eq!(
///     script,
///     "CREATE (n0:Person {name: 'Alice'})\nCREATE (n1:Person)\nCREATE (n0)-[:KNOWS]->(n1)\n",
/// );
/// let copy = build_graph_from_cypher(&script)?;
/// assert_eq!((copy.node_count(), copy.edge_count()), (2, 1));
/// # Ok::<(), petgraph_decypher::CypherError>(())
/// ```
///
/// [`build_graph_from_cypher`]: crate::build_graph_from_cypher
pub fn to_cypher_create<G: CypherGraph>(graph: &G) -> Result<String, CypherError> {
    let mut out = String::new();
    // Dense variable numbers, so the holes of a `StableGraph` do not show.
    let mut variables = HashMap::new();
    for (i, id) in graph.node_ids().enumerate() {
        let Some(node) = graph.node(id) else { continue };
        variables.insert(id, i);
        write!(out, "CREATE (n{i}").unwrap();
        for label in node.labels() {
            out.push(':');
            name(label, &mut out);
        }
        properties(node.properties(), &mut out)?;
        out.push_str(")\n");
    }
    for id in graph.edge_ids() {
        let (Some((source, target)), Some(edge)) = (graph.endpoints(id), graph.edge(id)) else {
            continue;
        };
        let (Some(s), Some(t)) = (variables.get(&source), variables.get(&target)) else {
            continue;
        };
        write!(out, "CREATE (n{s})-[:").unwrap();
        name(edge.rel_type(), &mut out);
        properties(edge.properties(), &mut out)?;
        writeln!(out, "]->(n{t})").unwrap();
    }
    Ok(out)
}

/// A label, relationship type or property key: bare if it is a plain
/// identifier, otherwise quoted in backticks.
fn name(name: &str, out: &mut String) {
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        out.push_str(name);
    } else {
        write!(out, "`{}`", name.replace('`', "``")).unwrap();
    }
}

/// ` {key: value, …}`, or nothing for no properties.
fn properties<'a>(
    props: impl Iterator<Item = (&'a str, &'a Value)>,
    out: &mut String,
) -> Result<(), CypherError> {
    let mut first = true;
    for (key, value) in props {
        out.push_str(if first { " {" } else { ", " });
        first = false;
        name(key, out);
        out.push_str(": ");
        literal(value, out)?;
    }
    if !first {
        out.push('}');
    }
    Ok(())
}

/// A Cypher expression that evaluates to `value`.
fn literal(value: &Value, out: &mut String) -> Result<(), CypherError> {
    match value {
        // Neither has a literal; these expressions evaluate to them.
        Value::Float(x) if x.is_nan() => out.push_str("0.0 / 0.0"),
        Value::Float(x) if x.is_infinite() => {
            out.push_str(if *x > 0.0 { "1.0 / 0.0" } else { "-1.0 / 0.0" });
        }
        // `-9223372036854775808` is a negated literal that overflows.
        Value::Integer(i64::MIN) => out.push_str("-9223372036854775807 - 1"),
        Value::String(s) => {
            // Cypher string literals cannot hold raw line breaks or other
            // control characters, so escape them.
            out.push('\'');
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\'' => out.push_str("\\'"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c.is_control() => write!(out, "\\U{:08X}", u32::from(c)).unwrap(),
                    c => out.push(c),
                }
            }
            out.push('\'');
        }
        Value::Null | Value::Boolean(_) | Value::Integer(_) | Value::Float(_) => {
            write!(out, "{value}").unwrap();
        }
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                literal(item, out)?;
            }
            out.push(']');
        }
        Value::Map(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                name(key, out);
                out.push_str(": ");
                literal(item, out)?;
            }
            out.push('}');
        }
        Value::Node(_) | Value::Relationship(_) | Value::Path(_) => {
            return Err(CypherError::TypeMismatch(format!(
                "a {} property has no Cypher literal",
                value.type_name()
            )));
        }
    }
    Ok(())
}
