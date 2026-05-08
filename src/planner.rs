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
    match plan_hir(input) {
        Ok(query) => Ok(query),
        Err(CypherError::ParseError(message))
            if message.contains("single-part query must end with RETURN") =>
        {
            plan_match_without_return(input)
        }
        Err(err) => Err(err),
    }
}

fn plan_hir(input: &str) -> Result<CypherQuery, CypherError> {
    let hir = cypher::analyze(input).map_err(|err| CypherError::ParseError(err.to_string()))?;
    PlanningContext::new(&hir).plan()
}

fn plan_match_without_return(input: &str) -> Result<CypherQuery, CypherError> {
    let mut query = plan_hir(&format!("{input} RETURN *"))?;
    if matches!(query.clauses.last(), Some(Clause::Return { .. })) {
        query.clauses.pop();
        Ok(query)
    } else {
        Err(CypherError::ParseError(
            "failed to normalize MATCH query without RETURN".into(),
        ))
    }
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
        items.iter()
            .map(|item| {
                Ok(ReturnItem {
                    expression: self.expression(item.expression)?,
                    alias: Some(self.binding_name(item.alias)?.to_string()),
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
                Ok(WhereExpr::Eq(self.expression(*left)?, self.literal_value(right)?))
            }
            other => Err(CypherError::Unsupported(format!(
                "unsupported WHERE expression: {other:?}"
            ))),
        }
    }

    fn expression(&self, expr_id: ExprId) -> Result<Expression, CypherError> {
        match &self.expr(expr_id)?.kind {
            ExprKind::Binding(binding) => Ok(Expression::Variable(self.binding_name(*binding)?.into())),
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

    fn properties(&self, expr_id: Option<ExprId>) -> Result<HashMap<String, CypherValue>, CypherError> {
        let Some(expr_id) = expr_id else {
            return Ok(HashMap::new());
        };

        match &self.expr(expr_id)?.kind {
            ExprKind::Map(entries) => entries
                .iter()
                .map(|(key, value)| Ok((self.property_key(*key)?.to_string(), self.literal_value(*value)?)))
                .collect(),
            other => Err(CypherError::Unsupported(format!(
                "unsupported property map expression: {other:?}"
            ))),
        }
    }

    fn node_pattern(&self, pattern: &hir::pattern::NodePattern) -> Result<NodePattern, CypherError> {
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

        let mut paths = Vec::new();
        let mut node_index = 0;
        let mut rel_index = 0;

        while node_index < pattern.nodes.len() {
            if rel_index >= pattern.relationships.len() {
                paths.push(PathPattern {
                    start: self.node_pattern(&pattern.nodes[node_index])?,
                    rels: Vec::new(),
                });
                node_index += 1;
                continue;
            }

            let remaining_nodes = pattern.nodes.len() - node_index;
            if remaining_nodes < 2 {
                return Err(CypherError::Unsupported(
                    "graph pattern is missing a node for a relationship".into(),
                ));
            }

            let remaining_rels = pattern.relationships.len() - rel_index;
            let path_len = remaining_rels.min(remaining_nodes - 1);
            let start = self.node_pattern(&pattern.nodes[node_index])?;
            let mut rels = Vec::with_capacity(path_len);

            for offset in 0..path_len {
                let current = node_index + offset;
                let next = current + 1;
                rels.push((
                    self.rel_pattern(&pattern.relationships[rel_index + offset])?,
                    self.node_pattern(&pattern.nodes[next])?,
                ));
            }

            paths.push(PathPattern { start, rels });
            node_index += path_len + 1;
            rel_index += path_len;
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
