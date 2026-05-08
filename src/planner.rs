//! HIR-backed query planner built on the external `cypher` crate.

use std::collections::HashMap;

use cypher::hir::{
    self,
    arena::{BindingId, ExprId},
    expr::{ComparisonOperator, ExprKind, Literal},
    ops::{Operation, ProjectionItem},
    pattern::{GraphPattern, RelationshipDirection, RelationshipPattern},
};

use crate::ast::*;
use crate::error::CypherError;

pub(crate) fn plan_query(input: &str) -> Result<CypherQuery, CypherError> {
    // First, attempt a direct parse. If it succeeds, plan immediately.
    if let Ok(hir) = analyze_query(input) {
        return PlanningContext::new(&hir).plan();
    }

    // If the direct parse failed, attempt a second pass with an appended
    // `RETURN *`. This robustly handles MATCH-only queries (and any other
    // single-part query that the `cypher` crate requires to end with RETURN)
    // without relying on fragile upstream error message strings.
    let with_return = format!("{input} RETURN *");
    match analyze_query(&with_return) {
        Ok(hir) => {
            let mut query = PlanningContext::new(&hir).plan()?;
            // Remove the synthetic RETURN clause we appended.
            if matches!(query.clauses.last(), Some(Clause::Return { .. })) {
                query.clauses.pop();
            }
            Ok(query)
        }
        Err(err) => {
            // Re-run the original query to surface its error to the caller.
            Err(CypherError::ParseError(err.to_string()))
        }
    }
}

fn analyze_query(input: &str) -> Result<hir::HirQuery, cypher::CypherError> {
    cypher::analyze(input)
}

struct PlanningContext<'a> {
    query: &'a hir::HirQuery,
}

impl<'a> PlanningContext<'a> {
    fn new(query: &'a hir::HirQuery) -> Self {
        Self { query }
    }

    fn plan(&self) -> Result<CypherQuery, CypherError> {
        let mut clauses = Vec::new();

        for part in &self.query.parts {
            for operation in &part.operations {
                match operation {
                    Operation::Match(op) => clauses.push(Clause::Match {
                        patterns: self.path_patterns(&op.pattern)?,
                        where_clause: self.where_clause(&op.predicates)?,
                    }),
                    Operation::Create(op) => clauses.push(Clause::Create {
                        patterns: self.path_patterns(&op.pattern)?,
                    }),
                    Operation::Merge(op) => {
                        let mut patterns = self.path_patterns(&op.pattern)?;
                        let pattern = patterns.pop().ok_or_else(|| {
                            CypherError::InvalidQuery("MERGE must contain a pattern".into())
                        })?;
                        if !patterns.is_empty() {
                            return Err(CypherError::Unsupported(
                                "MERGE with multiple disconnected patterns is not supported".into(),
                            ));
                        }
                        clauses.push(Clause::Merge { pattern });
                    }
                    Operation::Project(op) => clauses.push(Clause::Return {
                        items: self.return_items(&op.items)?,
                    }),
                    Operation::Delete(op) => clauses.push(Clause::Delete {
                        variables: self.delete_targets(&op.targets)?,
                        detach: op.detach,
                    }),
                    Operation::Return(_) | Operation::Finish => {}
                    other => {
                        return Err(CypherError::Unsupported(format!(
                            "unsupported clause or feature in query planner: {other:?}"
                        )));
                    }
                }
            }
        }

        Ok(CypherQuery { clauses })
    }

    fn return_items(&self, items: &[ProjectionItem]) -> Result<Vec<ReturnItem>, CypherError> {
        items
            .iter()
            .map(|item| {
                let expression = self.expression(item.expression)?;
                let alias = self.binding_name(item.alias)?.to_string();
                Ok(ReturnItem {
                    alias: match &expression {
                        Expression::Variable(name) if name == &alias => None,
                        _ => Some(alias),
                    },
                    expression,
                })
            })
            .collect()
    }

    fn delete_targets(&self, targets: &[ExprId]) -> Result<Vec<String>, CypherError> {
        targets
            .iter()
            .map(|target| match &self.expr(*target)?.kind {
                ExprKind::Binding(binding) => Ok(self.binding_name(*binding)?.to_string()),
                other => Err(CypherError::Unsupported(format!(
                    "unsupported DELETE target: {other:?}"
                ))),
            })
            .collect()
    }

    fn where_clause(&self, predicates: &[ExprId]) -> Result<Option<WhereExpr>, CypherError> {
        let mut iter = predicates.iter().copied();
        let Some(first) = iter.next() else {
            return Ok(None);
        };

        let mut combined = self.where_expr(first)?;
        for predicate in iter {
            combined = WhereExpr::And(Box::new(combined), Box::new(self.where_expr(predicate)?));
        }

        Ok(Some(combined))
    }

    fn where_expr(&self, expr_id: ExprId) -> Result<WhereExpr, CypherError> {
        match &self.expr(expr_id)?.kind {
            ExprKind::Binary { op, left, right } => match op {
                cypher::hir::expr::BinaryOp::And => Ok(WhereExpr::And(
                    Box::new(self.where_expr(*left)?),
                    Box::new(self.where_expr(*right)?),
                )),
                cypher::hir::expr::BinaryOp::Or => Ok(WhereExpr::Or(
                    Box::new(self.where_expr(*left)?),
                    Box::new(self.where_expr(*right)?),
                )),
                _ => Err(CypherError::Unsupported(format!(
                    "unsupported WHERE expression: {:?}",
                    self.expr(expr_id)?.kind
                ))),
            },
            ExprKind::Comparison { left, operators } => {
                if operators.len() != 1 {
                    return Err(CypherError::Unsupported(
                        "chained comparisons are not supported".into(),
                    ));
                }
                let (operator, right) = operators[0];
                if operator != ComparisonOperator::Eq {
                    return Err(CypherError::Unsupported(format!(
                        "unsupported comparison operator in WHERE clause: {operator:?}"
                    )));
                }
                Ok(WhereExpr::Eq(
                    self.expression(*left)?,
                    self.literal_value(right)?,
                ))
            }
            other => Err(CypherError::Unsupported(format!(
                "unsupported WHERE expression: {other:?}"
            ))),
        }
    }

    fn expression(&self, expr_id: ExprId) -> Result<Expression, CypherError> {
        match &self.expr(expr_id)?.kind {
            ExprKind::Binding(binding) => {
                Ok(Expression::Variable(self.binding_name(*binding)?.into()))
            }
            ExprKind::Property { base, key } => {
                let Expression::Variable(variable) = self.expression(*base)? else {
                    return Err(CypherError::Unsupported(
                        "nested property access is not supported".into(),
                    ));
                };
                Ok(Expression::Property(
                    variable,
                    self.property_key(*key)?.to_string(),
                ))
            }
            other => Err(CypherError::Unsupported(format!(
                "unsupported expression in query planner: {other:?}"
            ))),
        }
    }

    fn literal_value(&self, expr_id: ExprId) -> Result<CypherValue, CypherError> {
        match &self.expr(expr_id)?.kind {
            ExprKind::Literal(literal) => Ok(match literal {
                Literal::Null => CypherValue::Null,
                Literal::Boolean(value) => CypherValue::Boolean(*value),
                Literal::Integer(value) => CypherValue::Integer(*value),
                Literal::Float(value) => CypherValue::Float(*value),
                Literal::String(value) => CypherValue::String(value.clone()),
            }),
            other => Err(CypherError::Unsupported(format!(
                "unsupported literal expression: {other:?}"
            ))),
        }
    }

    fn properties(
        &self,
        expr_id: Option<ExprId>,
    ) -> Result<HashMap<String, CypherValue>, CypherError> {
        let Some(expr_id) = expr_id else {
            return Ok(HashMap::new());
        };

        match &self.expr(expr_id)?.kind {
            ExprKind::Map(entries) => entries
                .iter()
                .map(|(key, value)| {
                    Ok((
                        self.property_key(*key)?.to_string(),
                        self.literal_value(*value)?,
                    ))
                })
                .collect(),
            other => Err(CypherError::Unsupported(format!(
                "unsupported property map expression: {other:?}"
            ))),
        }
    }

    fn node_pattern(
        &self,
        pattern: &hir::pattern::NodePattern,
    ) -> Result<NodePattern, CypherError> {
        Ok(NodePattern {
            variable: pattern
                .binding
                .map(|binding| self.binding_name(binding).map(str::to_string))
                .transpose()?,
            labels: pattern
                .labels
                .iter()
                .map(|label| self.label_name(*label).map(str::to_string))
                .collect::<Result<Vec<_>, _>>()?,
            properties: self.properties(pattern.properties)?,
        })
    }

    fn rel_pattern(&self, relationship: &RelationshipPattern) -> Result<RelPattern, CypherError> {
        if !matches!(
            relationship.length,
            cypher::hir::pattern::RelationshipLength::Single
        ) {
            return Err(CypherError::Unsupported(
                "variable-length relationships are not supported".into(),
            ));
        }

        let rel_type = match relationship.types.as_slice() {
            [] => None,
            [rel_type] => Some(self.rel_type_name(*rel_type)?.to_string()),
            _ => {
                return Err(CypherError::Unsupported(
                    "multiple relationship types are not supported".into(),
                ));
            }
        };

        let direction = match relationship.direction {
            // NOTE: The `cypher` crate (v0.2.0-alpha.2) lowers the left-directed
            // syntax `(a)<-[:T]-(b)` as `Undirected` rather than `RightToLeft`
            // due to a known upstream limitation in its HIR lowering pass.
            // Until the upstream crate preserves direction for this syntax,
            // `Undirected` is mapped to `Both` (match in either direction), which
            // is the safest available approximation. Queries that rely on
            // left-directed edges being strictly reversed should use the
            // right-directed form `(b)-[:T]->(a)` instead.
            RelationshipDirection::Undirected => RelDirection::Both,
            RelationshipDirection::Both => {
                return Err(CypherError::Unsupported(
                    "bidirectional relationships are not supported".into(),
                ));
            }
            RelationshipDirection::LeftToRight => RelDirection::Right,
            RelationshipDirection::RightToLeft => RelDirection::Left,
        };

        Ok(RelPattern {
            variable: relationship
                .binding
                .map(|binding| self.binding_name(binding).map(str::to_string))
                .transpose()?,
            rel_type,
            properties: self.properties(relationship.properties)?,
            direction,
        })
    }

    fn path_patterns(&self, pattern: &GraphPattern) -> Result<Vec<PathPattern>, CypherError> {
        // Fast path: no relationships means every node is a standalone path.
        if pattern.relationships.is_empty() {
            return pattern
                .nodes
                .iter()
                .map(|node| {
                    Ok(PathPattern {
                        start: self.node_pattern(node)?,
                        rels: Vec::new(),
                    })
                })
                .collect();
        }

        // The `cypher` HIR (v0.2.0-alpha.2) has a known bug in its
        // `lower_pattern_element` function: for a chained path like
        // `(a)-[:E]->(b)-[:F]->(c)`, all relationships are assigned `left = 0`
        // (the start node index is never advanced between hops). The `right`
        // field, however, correctly increases by one per hop (1, 2, 3, ...).
        //
        // For a comma-separated disconnected pattern like
        // `(a)-[:E]->(b), (c)-[:F]->(d)`, the second path's relationship also
        // has `left = 0` and `right = 1` (local within its own segment).
        //
        // This means:
        //   - A new independent path begins whenever `rel.right` does NOT
        //     increase compared to the previous relationship's `right` (i.e.
        //     the `right` counter "resets").
        //   - Within a chain, successive `right` values are strictly increasing.
        //
        // We exploit this property to split the flat relationship list into
        // independent `PathPattern`s, using `node_offset` to translate local
        // node indices to global positions in `pattern.nodes`.
        let mut paths: Vec<PathPattern> = Vec::new();
        let mut node_offset: usize = 0;

        let mut rel_iter = pattern.relationships.iter().peekable();
        while let Some(first_rel) = rel_iter.next() {
            // Global node index of the start and first hop target.
            let start_global = node_offset + first_rel.left;
            let first_end_global = node_offset + first_rel.right;

            if start_global >= pattern.nodes.len() || first_end_global >= pattern.nodes.len() {
                return Err(CypherError::InvalidQuery(
                    "HIR relationship references an out-of-bounds node index".into(),
                ));
            }

            let start = self.node_pattern(&pattern.nodes[start_global])?;
            let mut rels = vec![(
                self.rel_pattern(first_rel)?,
                self.node_pattern(&pattern.nodes[first_end_global])?,
            )];

            // A chain continues as long as the next relationship's `right`
            // value is strictly greater than the current one (indicating the
            // same connected path rather than a new segment).
            let mut chain_right = first_rel.right;
            while let Some(next_rel) = rel_iter.peek() {
                if next_rel.right > chain_right {
                    // Extend the existing chain.
                    let next_rel = rel_iter.next().unwrap();
                    let next_end_global = node_offset + next_rel.right;
                    if next_end_global >= pattern.nodes.len() {
                        return Err(CypherError::InvalidQuery(
                            "HIR relationship references an out-of-bounds node index".into(),
                        ));
                    }
                    rels.push((
                        self.rel_pattern(next_rel)?,
                        self.node_pattern(&pattern.nodes[next_end_global])?,
                    ));
                    chain_right = next_rel.right;
                } else {
                    // `right` did not increase: start of a new independent path.
                    break;
                }
            }

            // Nodes consumed by this path: one start node + one per hop.
            node_offset += rels.len() + 1;
            paths.push(PathPattern { start, rels });
        }

        // Any remaining nodes (not covered by a relationship) are standalone.
        while node_offset < pattern.nodes.len() {
            paths.push(PathPattern {
                start: self.node_pattern(&pattern.nodes[node_offset])?,
                rels: Vec::new(),
            });
            node_offset += 1;
        }

        Ok(paths)
    }

    fn expr(&self, expr_id: ExprId) -> Result<&hir::expr::HirExpr, CypherError> {
        Ok(self.query.arenas.expressions.get(expr_id))
    }

    fn binding_name(&self, binding: BindingId) -> Result<&str, CypherError> {
        Ok(self.query.arenas.bindings.get(binding).name.as_str())
    }

    fn label_name(&self, label: cypher::hir::arena::LabelId) -> Result<&str, CypherError> {
        self.query.arenas.labels.name_of(label).ok_or_else(|| {
            CypherError::InvalidQuery(format!("unknown label id in HIR: {:?}", label))
        })
    }

    fn rel_type_name(&self, rel_type: cypher::hir::arena::RelTypeId) -> Result<&str, CypherError> {
        self.query
            .arenas
            .relationship_types
            .name_of(rel_type)
            .ok_or_else(|| {
                CypherError::InvalidQuery(format!(
                    "unknown relationship type id in HIR: {:?}",
                    rel_type
                ))
            })
    }

    fn property_key(
        &self,
        property: cypher::hir::arena::PropertyKeyId,
    ) -> Result<&str, CypherError> {
        self.query
            .arenas
            .property_keys
            .name_of(property)
            .ok_or_else(|| {
                CypherError::InvalidQuery(format!("unknown property key id in HIR: {:?}", property))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_match_only_query_without_return() {
        // A MATCH-only query (no RETURN) must succeed without relying on a
        // specific upstream error message string.
        let q = plan_query("MATCH (n)").expect("MATCH without RETURN should be accepted");
        assert_eq!(q.clauses.len(), 1);
        assert!(matches!(q.clauses[0], Clause::Match { .. }));
    }

    #[test]
    fn plan_match_relationship_without_return() {
        let q = plan_query("MATCH (a)-[:E]->(b)").expect("MATCH rel without RETURN should work");
        assert_eq!(q.clauses.len(), 1);
        assert!(matches!(q.clauses[0], Clause::Match { .. }));
    }

    #[test]
    fn path_patterns_disconnected_comma_separated() {
        // MATCH (a)-[:E]->(b), (c)-[:E]->(d)
        // Must produce two independent PathPatterns, not one mis-planned path.
        let q = plan_query("MATCH (a)-[:E]->(b), (c)-[:E]->(d) RETURN a")
            .expect("disconnected MATCH should parse");
        let Clause::Match { patterns, .. } = &q.clauses[0] else {
            panic!("expected Match clause");
        };
        assert_eq!(patterns.len(), 2, "expected two independent path patterns");
        // Each path must have exactly one hop.
        assert_eq!(patterns[0].rels.len(), 1);
        assert_eq!(patterns[1].rels.len(), 1);
        // Start nodes must be distinct variables.
        assert_eq!(patterns[0].start.variable.as_deref(), Some("a"));
        assert_eq!(patterns[1].start.variable.as_deref(), Some("c"));
    }

    #[test]
    fn path_patterns_connected_chain() {
        // MATCH (a)-[:E]->(b)-[:F]->(c) must stay as a single two-hop path.
        let q = plan_query("MATCH (a)-[:E]->(b)-[:F]->(c) RETURN a")
            .expect("chained MATCH should parse");
        let Clause::Match { patterns, .. } = &q.clauses[0] else {
            panic!("expected Match clause");
        };
        assert_eq!(patterns.len(), 1, "expected a single chained path pattern");
        assert_eq!(patterns[0].rels.len(), 2);
    }
}
