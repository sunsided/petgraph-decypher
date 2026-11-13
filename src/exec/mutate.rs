//! Graph access for the executor, and the updating clauses.

use decypher::hir::ops::{MergeOp, RemoveItem, SetItem};
use decypher::hir::{ExprKind, GraphPattern, RelationshipDirection};
use petgraph::graph::{EdgeIndex, NodeIndex};

use super::{Executor, Result, Row};
use crate::element::{CypherEdge, CypherEdgeMut, CypherNode, CypherNodeMut};
use crate::error::CypherError;
use crate::graph::{CypherGraph, CypherGraphMut};
use crate::prepared::Unit;
use crate::value::{Properties, Value};

/// A node or relationship being updated.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Element {
    Node(NodeIndex),
    Edge(EdgeIndex),
}

/// How the executor reaches the graph: read-only for [`Query::execute`],
/// read-write for [`Query::execute_mut`].
///
/// [`Query::execute`]: crate::Query::execute
/// [`Query::execute_mut`]: crate::Query::execute_mut
pub(crate) trait Access {
    type G: CypherGraph;
    fn graph(&self) -> &Self::G;
    fn create_node(
        &mut self,
        variable: Option<&str>,
        labels: Vec<String>,
        properties: Properties,
    ) -> Result<NodeIndex>;
    fn create_edge(
        &mut self,
        variable: Option<&str>,
        source: NodeIndex,
        target: NodeIndex,
        rel_type: String,
        properties: Properties,
    ) -> Result<EdgeIndex>;
    fn set_property(&mut self, element: Element, key: &str, value: Value) -> Result<()>;
    fn clear_properties(&mut self, element: Element) -> Result<()>;
    fn set_label(&mut self, node: NodeIndex, label: &str, present: bool) -> Result<()>;
    fn remove(&mut self, nodes: Vec<NodeIndex>, edges: Vec<EdgeIndex>) -> Result<()>;
    /// Reverts every change made so far, after a failed query.
    fn rollback(&mut self);
}

pub(crate) struct ReadOnly<'g, G>(pub &'g G);

/// Write access that logs how to undo each change, so a query that fails
/// leaves the graph as it was.
pub(crate) struct ReadWrite<'g, G> {
    graph: &'g mut G,
    undo: Vec<Undo>,
}

impl<'g, G> ReadWrite<'g, G> {
    pub(crate) fn new(graph: &'g mut G) -> Self {
        Self {
            graph,
            undo: Vec::new(),
        }
    }
}

/// The inverse of one change.
enum Undo {
    Node(NodeIndex),
    Edge(EdgeIndex),
    Property(Element, String, Option<Value>),
    Properties(Element, Properties),
    Label(NodeIndex, String, bool),
}

fn read_only<T>() -> Result<T> {
    Err(CypherError::InvalidQuery(
        "the query modifies the graph; run it with a mutable graph".into(),
    ))
}

impl<G: CypherGraph> Access for ReadOnly<'_, G> {
    type G = G;

    fn graph(&self) -> &G {
        self.0
    }

    fn create_node(&mut self, _: Option<&str>, _: Vec<String>, _: Properties) -> Result<NodeIndex> {
        read_only()
    }

    fn create_edge(
        &mut self,
        _: Option<&str>,
        _: NodeIndex,
        _: NodeIndex,
        _: String,
        _: Properties,
    ) -> Result<EdgeIndex> {
        read_only()
    }

    fn set_property(&mut self, _: Element, _: &str, _: Value) -> Result<()> {
        read_only()
    }

    fn clear_properties(&mut self, _: Element) -> Result<()> {
        read_only()
    }

    fn set_label(&mut self, _: NodeIndex, _: &str, _: bool) -> Result<()> {
        read_only()
    }

    fn remove(&mut self, _: Vec<NodeIndex>, _: Vec<EdgeIndex>) -> Result<()> {
        read_only()
    }

    fn rollback(&mut self) {}
}

fn missing(element: Element) -> CypherError {
    CypherError::RuntimeError(format!("{element:?} does not exist"))
}

impl<G: CypherGraphMut> ReadWrite<'_, G> {
    fn properties_of(&self, element: Element) -> Result<Properties> {
        let collect = |it: &mut dyn Iterator<Item = (&str, &Value)>| -> Properties {
            it.map(|(k, v)| (k.to_owned(), v.clone())).collect()
        };
        match element {
            Element::Node(n) => self.graph.node(n).map(|w| collect(&mut w.properties())),
            Element::Edge(e) => self.graph.edge(e).map(|w| collect(&mut w.properties())),
        }
        .ok_or(missing(element))
    }

    fn property_of(&self, element: Element, key: &str) -> Result<Option<Value>> {
        match element {
            Element::Node(n) => self.graph.node(n).map(|w| w.property(key).cloned()),
            Element::Edge(e) => self.graph.edge(e).map(|w| w.property(key).cloned()),
        }
        .ok_or(missing(element))
    }

    fn write_property(&mut self, element: Element, key: &str, value: Value) -> Result<()> {
        match element {
            Element::Node(n) => self
                .graph
                .node_mut(n)
                .ok_or(missing(element))?
                .set_property(key, value),
            Element::Edge(e) => self
                .graph
                .edge_mut(e)
                .ok_or(missing(element))?
                .set_property(key, value),
        }
        Ok(())
    }

    fn write_label(&mut self, node: NodeIndex, label: &str, present: bool) -> Result<()> {
        let weight = self
            .graph
            .node_mut(node)
            .ok_or(missing(Element::Node(node)))?;
        if present {
            weight.add_label(label);
        } else {
            weight.remove_label(label);
        }
        Ok(())
    }

    fn write_clear(&mut self, element: Element) -> Result<()> {
        match element {
            Element::Node(n) => self
                .graph
                .node_mut(n)
                .ok_or(missing(element))?
                .clear_properties(),
            Element::Edge(e) => self
                .graph
                .edge_mut(e)
                .ok_or(missing(element))?
                .clear_properties(),
        }
        Ok(())
    }
}

impl<G: CypherGraphMut> Access for ReadWrite<'_, G> {
    type G = G;

    fn graph(&self) -> &G {
        self.graph
    }

    fn create_node(
        &mut self,
        variable: Option<&str>,
        labels: Vec<String>,
        properties: Properties,
    ) -> Result<NodeIndex> {
        let id = self
            .graph
            .add_node(G::Node::from_cypher(variable, labels, properties));
        self.undo.push(Undo::Node(id));
        Ok(id)
    }

    fn create_edge(
        &mut self,
        variable: Option<&str>,
        source: NodeIndex,
        target: NodeIndex,
        rel_type: String,
        properties: Properties,
    ) -> Result<EdgeIndex> {
        let id = self.graph.add_edge(
            source,
            target,
            G::Edge::from_cypher(variable, rel_type, properties),
        );
        self.undo.push(Undo::Edge(id));
        Ok(id)
    }

    fn set_property(&mut self, element: Element, key: &str, value: Value) -> Result<()> {
        let old = self.property_of(element, key)?;
        self.write_property(element, key, value)?;
        self.undo.push(Undo::Property(element, key.to_owned(), old));
        Ok(())
    }

    fn clear_properties(&mut self, element: Element) -> Result<()> {
        let old = self.properties_of(element)?;
        self.write_clear(element)?;
        self.undo.push(Undo::Properties(element, old));
        Ok(())
    }

    fn set_label(&mut self, node: NodeIndex, label: &str, present: bool) -> Result<()> {
        let had = self
            .graph
            .node(node)
            .ok_or(missing(Element::Node(node)))?
            .has_label(label);
        self.write_label(node, label, present)?;
        self.undo.push(Undo::Label(node, label.to_owned(), had));
        Ok(())
    }

    fn remove(&mut self, nodes: Vec<NodeIndex>, edges: Vec<EdgeIndex>) -> Result<()> {
        self.graph.remove(nodes, edges);
        self.undo.clear();
        Ok(())
    }

    /// Undoes changes newest first. Deletions are only applied once the query
    /// has succeeded, so every element created by the query is still the
    /// newest of its kind when its creation is undone, and removing it moves
    /// no other element.
    fn rollback(&mut self) {
        while let Some(undo) = self.undo.pop() {
            // The elements were valid when logged; failures cannot occur.
            let _ = match undo {
                Undo::Node(n) => {
                    self.graph.remove(vec![n], Vec::new());
                    Ok(())
                }
                Undo::Edge(e) => {
                    self.graph.remove(Vec::new(), vec![e]);
                    Ok(())
                }
                Undo::Property(el, key, old) => {
                    self.write_property(el, &key, old.unwrap_or_default())
                }
                Undo::Properties(el, old) => self.write_clear(el).and_then(|()| {
                    old.into_iter()
                        .try_for_each(|(k, v)| self.write_property(el, &k, v))
                }),
                Undo::Label(n, label, had) => self.write_label(n, &label, had),
            };
        }
    }
}

/// Checks that every value of a property map can be stored, dropping `null`s.
fn storable(properties: Properties) -> Result<Properties> {
    properties
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| {
            if v.is_property_value() {
                Ok((k, v))
            } else {
                Err(CypherError::TypeMismatch(format!(
                    "property `{k}` cannot store a {}",
                    v.type_name()
                )))
            }
        })
        .collect()
}

impl<'q, A: Access> Executor<'q, A> {
    /// `CREATE`: creates every unbound node and every relationship of the
    /// pattern, binding them in `row`.
    pub(super) fn create_pattern(&mut self, pattern: &GraphPattern, row: &mut Row) -> Result<()> {
        // Reject what cannot be created before creating anything. Only plain
        // labels are created: dropping `:A|B` or `:$(expr)` would silently
        // create a node without them.
        if pattern.nodes.iter().any(|n| n.label_expression.is_some()) {
            return Err(CypherError::Unsupported(
                "label expressions and dynamic labels in CREATE / MERGE".into(),
            ));
        }
        for rel in &pattern.relationships {
            if rel.types.len() != 1 {
                return Err(CypherError::InvalidQuery(
                    "CREATE needs exactly one type for every relationship".into(),
                ));
            }
        }
        let mut nodes = Vec::with_capacity(pattern.nodes.len());
        for node in &pattern.nodes {
            let bound = node.binding.and_then(|b| row[b.0].clone());
            let id = match bound {
                // A bound node may come from a parameter, so it must exist
                // and not be pending deletion.
                Some(Value::Node(id)) if self.graph().node(id).is_some() && self.node_alive(id) => {
                    id
                }
                Some(Value::Node(id)) => {
                    return Err(CypherError::RuntimeError(format!(
                        "cannot create a relationship to node {} that does not exist",
                        id.index()
                    )));
                }
                Some(other) => {
                    return Err(CypherError::TypeMismatch(format!(
                        "cannot create a relationship to a {}",
                        other.type_name()
                    )));
                }
                None => {
                    let labels = node
                        .labels
                        .iter()
                        .map(|l| self.q.labels[l.0].clone())
                        .collect();
                    let properties = storable(self.properties(node.properties, row)?)?;
                    let variable = node.binding.map(|b| self.binding_name(b));
                    let id = self.access.create_node(variable, labels, properties)?;
                    if let Some(b) = node.binding {
                        row[b.0] = Some(Value::Node(id));
                    }
                    id
                }
            };
            nodes.push(id);
        }

        let mut rels = Vec::with_capacity(pattern.relationships.len());
        for rel in &pattern.relationships {
            let [rel_type] = rel.types.as_slice() else {
                return Err(CypherError::InvalidQuery(
                    "CREATE needs exactly one type for every relationship".into(),
                ));
            };
            // `CREATE` requires a direction; an undirected `MERGE` creates
            // its relationship left to right.
            let (source, target) = match rel.direction {
                RelationshipDirection::RightToLeft => (nodes[rel.right], nodes[rel.left]),
                _ => (nodes[rel.left], nodes[rel.right]),
            };
            let properties = storable(self.properties(rel.properties, row)?)?;
            let variable = rel.binding.map(|b| self.binding_name(b));
            let id = self.access.create_edge(
                variable,
                source,
                target,
                self.q.rel_types[rel_type.0].clone(),
                properties,
            )?;
            if let Some(b) = rel.binding {
                row[b.0] = Some(Value::Relationship(id));
            }
            rels.push(id);
        }

        for path in &pattern.path_bindings {
            let value = Value::Path(crate::value::Path {
                nodes: path.nodes.iter().map(|&i| nodes[i]).collect(),
                relationships: path.relationships.iter().map(|&i| rels[i]).collect(),
            });
            row[path.binding.0] = Some(value);
        }
        Ok(())
    }

    /// The query outlives the executor's borrows, so names from it can be
    /// held while the graph is mutated.
    fn binding_name(&self, b: decypher::hir::BindingId) -> &'q str {
        let q: &'q Unit = self.q;
        &q.hir.arenas.bindings.get(b).name
    }

    fn key(&self, key: decypher::hir::arena::PropertyKeyId) -> &'q str {
        let q: &'q Unit = self.q;
        &q.keys[key.0]
    }

    fn label(&self, label: decypher::hir::arena::LabelId) -> &'q str {
        let q: &'q Unit = self.q;
        &q.labels[label.0]
    }

    /// `MERGE`: the matches of the pattern, or the pattern created once.
    pub(super) fn merge(&mut self, m: &MergeOp, row: Row) -> Result<Vec<Row>> {
        // A `null` property never matches, so MERGE would create a new element
        // on every run.
        let property_maps = (m.pattern.nodes.iter().map(|n| n.properties))
            .chain(m.pattern.relationships.iter().map(|r| r.properties));
        for expr in property_maps {
            if let Some((key, _)) = self
                .properties(expr, &row)?
                .iter()
                .find(|(_, v)| v.is_null())
            {
                return Err(CypherError::RuntimeError(format!(
                    "cannot MERGE with a null value for property `{key}`"
                )));
            }
        }
        let matches = self.match_pattern(&m.pattern, &row, &[], None)?;
        if matches.is_empty() {
            let mut row = row;
            self.create_pattern(&m.pattern, &mut row)?;
            self.set_items(&m.on_create, &row)?;
            return Ok(vec![row]);
        }
        for row in &matches {
            self.set_items(&m.on_match, row)?;
        }
        Ok(matches)
    }

    fn element(&self, value: Value, clause: &str) -> Result<Option<Element>> {
        match value {
            Value::Null => Ok(None),
            Value::Node(n) => Ok(Some(Element::Node(n))),
            Value::Relationship(e) => Ok(Some(Element::Edge(e))),
            other => Err(CypherError::TypeMismatch(format!(
                "{clause} expects a node or relationship, got {}",
                other.type_name()
            ))),
        }
    }

    fn set_value(&mut self, element: Element, key: &str, value: Value) -> Result<()> {
        if !value.is_property_value() {
            return Err(CypherError::TypeMismatch(format!(
                "property `{key}` cannot store a {}",
                value.type_name()
            )));
        }
        self.access.set_property(element, key, value)
    }

    /// The properties `SET n = x` / `SET n += x` copy: a map, or the
    /// properties of a node or relationship.
    fn property_map(&self, value: Value) -> Result<Option<Properties>> {
        match value {
            Value::Null => Ok(None),
            Value::Map(map) => Ok(Some(map)),
            Value::Node(_) | Value::Relationship(_) => Ok(self.element_properties(&value)),
            other => Err(CypherError::TypeMismatch(format!(
                "SET expects a map, got {}",
                other.type_name()
            ))),
        }
    }

    pub(super) fn set_items(&mut self, items: &[SetItem], row: &Row) -> Result<()> {
        for item in items {
            match item {
                SetItem::SetProperty { target, value } => {
                    let ExprKind::Property { base, key } =
                        &self.q.hir.arenas.expressions.get(*target).kind
                    else {
                        return Err(CypherError::InvalidQuery(
                            "SET needs a property as target".into(),
                        ));
                    };
                    let base = self.eval(*base, row)?;
                    let value = self.eval(*value, row)?;
                    if let Some(element) = self.element(base, "SET")? {
                        let key = self.key(*key);
                        self.set_value(element, key, value)?;
                    }
                }
                SetItem::SetDynamicProperty { entity, key, value } => {
                    let base = self.eval(*entity, row)?;
                    let key = match self.eval(*key, row)? {
                        Value::String(k) => k,
                        other => {
                            return Err(CypherError::TypeMismatch(format!(
                                "a dynamic property key must be a string, got {}",
                                other.type_name()
                            )));
                        }
                    };
                    let value = self.eval(*value, row)?;
                    if let Some(element) = self.element(base, "SET")? {
                        self.set_value(element, &key, value)?;
                    }
                }
                SetItem::SetVariable { target, value }
                | SetItem::ReplaceProperties {
                    entity: target,
                    value,
                } => {
                    let base = row[target.0].clone().unwrap_or_default();
                    let map = self.eval(*value, row)?;
                    let Some(element) = self.element(base, "SET")? else {
                        continue;
                    };
                    let Some(map) = self.property_map(map)? else {
                        continue;
                    };
                    // Validate the whole map before dropping the old properties.
                    let map = storable(map)?;
                    self.access.clear_properties(element)?;
                    for (k, v) in map {
                        self.set_value(element, &k, v)?;
                    }
                }
                SetItem::MergeProperties { entity, value } => {
                    let base = row[entity.0].clone().unwrap_or_default();
                    let map = self.eval(*value, row)?;
                    let Some(element) = self.element(base, "SET")? else {
                        continue;
                    };
                    let Some(map) = self.property_map(map)? else {
                        continue;
                    };
                    for (k, v) in map {
                        self.set_value(element, &k, v)?;
                    }
                }
                SetItem::SetLabels { node, labels } => {
                    match row[node.0].clone().unwrap_or_default() {
                        Value::Null => {}
                        Value::Node(n) => {
                            for l in labels {
                                let label = self.label(*l);
                                self.access.set_label(n, label, true)?;
                            }
                        }
                        other => {
                            return Err(CypherError::TypeMismatch(format!(
                                "SET labels expects a node, got {}",
                                other.type_name()
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn remove_items(&mut self, items: &[RemoveItem], row: &Row) -> Result<()> {
        for item in items {
            match item {
                RemoveItem::Property { target } => {
                    let ExprKind::Property { base, key } =
                        &self.q.hir.arenas.expressions.get(*target).kind
                    else {
                        return Err(CypherError::InvalidQuery(
                            "REMOVE needs a property as target".into(),
                        ));
                    };
                    let base = self.eval(*base, row)?;
                    if let Some(element) = self.element(base, "REMOVE")? {
                        let key = self.key(*key);
                        self.access.set_property(element, key, Value::Null)?;
                    }
                }
                RemoveItem::Labels { node, labels } => {
                    match row[node.0].clone().unwrap_or_default() {
                        Value::Null => {}
                        Value::Node(n) => {
                            for l in labels {
                                let label = self.label(*l);
                                self.access.set_label(n, label, false)?;
                            }
                        }
                        other => {
                            return Err(CypherError::TypeMismatch(format!(
                                "REMOVE labels expects a node, got {}",
                                other.type_name()
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
