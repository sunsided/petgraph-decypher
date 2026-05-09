//! HIR-backed query planner built on the external `cypher` crate.

use std::collections::HashMap;

use decypher::hir::{
    self,
    arena::{BindingId, ExprId},
    expr::{BinaryOp as HirBinaryOp, ComparisonOperator, ExprKind, Literal, UnaryOp as HirUnaryOp},
    ops::{Operation, ProjectionItem},
    pattern::{GraphPattern, RelationshipDirection, RelationshipPattern},
};

use crate::ast::*;
use crate::error::CypherError;

pub(crate) fn plan_query(input: &str) -> Result<CypherQuery, CypherError> {
    // First, attempt a direct parse. If it succeeds, plan immediately.
    let first_err = match analyze_query(input) {
        Ok(hir) => return PlanningContext::new(&hir).plan(),
        Err(e) => e,
    };

    // If the direct parse failed, attempt a second pass with an appended
    // `RETURN *`. This robustly handles MATCH-only queries (and any other
    // single-part query that the `cypher` crate requires to end with RETURN)
    // without relying on fragile upstream error message strings.
    let trimmed = input.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
    let with_return = format!("{trimmed} RETURN *");
    match analyze_query(&with_return) {
        Ok(hir) => {
            let mut query = PlanningContext::new(&hir).plan()?;
            // Remove the synthetic RETURN clause we appended.
            if matches!(query.clauses.last(), Some(Clause::Return { .. })) {
                query.clauses.pop();
            }
            Ok(query)
        }
        Err(_) => {
            // Return the original error so the caller sees the correct query.
            Err(CypherError::ParseError(first_err.to_string()))
        }
    }
}

fn analyze_query(input: &str) -> Result<hir::HirQuery, decypher::CypherError> {
    decypher::analyze(input)
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
                    Operation::OptionalMatch(op) => clauses.push(Clause::OptionalMatch {
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
                        let on_create = self.set_items(&op.on_create)?;
                        let on_match = self.set_items(&op.on_match)?;
                        clauses.push(Clause::Merge {
                            pattern,
                            on_create,
                            on_match,
                        });
                    }
                    Operation::Project(op) => clauses.push(Clause::Return {
                        items: self.return_items(&op.items)?,
                        distinct: op.distinct,
                    }),
                    Operation::Return(_) | Operation::Finish => {}
                    Operation::Delete(op) => clauses.push(Clause::Delete {
                        variables: self.delete_targets(&op.targets)?,
                        detach: op.detach,
                    }),
                    Operation::Sort(op) => {
                        let items = op
                            .items
                            .iter()
                            .map(|item| {
                                Ok(SortItem {
                                    expression: self.expression(item.expression)?,
                                    direction: match item.direction {
                                        hir::ops::SortDirection::Ascending => SortDirection::Asc,
                                        hir::ops::SortDirection::Descending => SortDirection::Desc,
                                    },
                                })
                            })
                            .collect::<Result<Vec<_>, CypherError>>()?;
                        clauses.push(Clause::OrderBy { items });
                    }
                    Operation::Skip(op) => {
                        let count = self.usize_from_expr(op.count)?;
                        clauses.push(Clause::Skip { count });
                    }
                    Operation::Limit(op) => {
                        let count = self.usize_from_expr(op.count)?;
                        clauses.push(Clause::Limit { count });
                    }
                    Operation::Unwind(op) => {
                        clauses.push(Clause::Unwind {
                            expression: self.expression(op.expression)?,
                            variable: self.binding_name(op.variable)?.to_string(),
                        });
                    }
                    Operation::Set(op) => {
                        clauses.push(Clause::Set {
                            items: self.set_items(&op.items)?,
                        });
                    }
                    Operation::Remove(op) => {
                        clauses.push(Clause::Remove {
                            items: self.remove_items(&op.items)?,
                        });
                    }
                    Operation::Filter(op) => {
                        // Standalone WHERE filter (already handled inside Match)
                        // This can appear after WITH or in subqueries.
                        // For now, we wrap it in a synthetic Match or handle inline.
                        // Since our Clause model doesn't have a standalone Filter,
                        // we ignore it here — it's typically part of Match/OptionalMatch.
                        let _ = op;
                    }
                    Operation::Aggregate(op) => {
                        // Aggregate is handled as a special Return/With variant
                        let mut items = Vec::new();
                        for key in &op.grouping_keys {
                            items.push(self.return_item(key)?);
                        }
                        for agg in &op.aggregates {
                            let name = self.function_name(agg.function)?.to_string();
                            let args = agg
                                .args
                                .iter()
                                .map(|&arg| self.expression(arg))
                                .collect::<Result<Vec<_>, _>>()?;
                            let alias = self.binding_name(agg.alias)?.to_string();
                            items.push(ReturnItem {
                                expression: Expression::FunctionCall {
                                    name,
                                    args,
                                    distinct: agg.distinct,
                                },
                                alias: Some(alias),
                            });
                        }
                        clauses.push(Clause::Return {
                            items,
                            distinct: false,
                        });
                    }
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
        items.iter().map(|item| self.return_item(item)).collect()
    }

    fn return_item(&self, item: &ProjectionItem) -> Result<ReturnItem, CypherError> {
        let expression = self.expression(item.expression)?;
        let alias = self.binding_name(item.alias)?.to_string();
        Ok(ReturnItem {
            alias: match &expression {
                Expression::Variable(name) if name == &alias => None,
                _ => Some(alias),
            },
            expression,
        })
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
                HirBinaryOp::And => Ok(WhereExpr::And(
                    Box::new(self.where_expr(*left)?),
                    Box::new(self.where_expr(*right)?),
                )),
                HirBinaryOp::Or => Ok(WhereExpr::Or(
                    Box::new(self.where_expr(*left)?),
                    Box::new(self.where_expr(*right)?),
                )),
                HirBinaryOp::Xor => Ok(WhereExpr::Xor(
                    Box::new(self.where_expr(*left)?),
                    Box::new(self.where_expr(*right)?),
                )),
                HirBinaryOp::Eq => Ok(WhereExpr::Eq(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::Ne => Ok(WhereExpr::NotEq(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::Lt => Ok(WhereExpr::Lt(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::Gt => Ok(WhereExpr::Gt(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::Le => Ok(WhereExpr::Le(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::Ge => Ok(WhereExpr::Ge(
                    self.expression(*left)?,
                    self.expression(*right)?,
                )),
                HirBinaryOp::StartsWith => Ok(WhereExpr::StartsWith(
                    self.expression(*left)?,
                    self.literal_string(*right)?,
                )),
                HirBinaryOp::EndsWith => Ok(WhereExpr::EndsWith(
                    self.expression(*left)?,
                    self.literal_string(*right)?,
                )),
                HirBinaryOp::Contains => Ok(WhereExpr::Contains(
                    self.expression(*left)?,
                    self.literal_string(*right)?,
                )),
                HirBinaryOp::In => {
                    let lhs = self.expression(*left)?;
                    let rhs = self.expression(*right)?;
                    match rhs {
                        Expression::List(items) => {
                            let values = items
                                .into_iter()
                                .map(|expr| match expr {
                                    Expression::Literal(v) => Ok(v),
                                    other => Err(CypherError::Unsupported(format!(
                                        "IN list must contain literals, got: {:?}",
                                        other
                                    ))),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok(WhereExpr::In(lhs, values))
                        }
                        Expression::Literal(CypherValue::List(values)) => {
                            Ok(WhereExpr::In(lhs, values))
                        }
                        other => Err(CypherError::Unsupported(format!(
                            "IN requires a list literal, got: {:?}",
                            other
                        ))),
                    }
                }
                _ => Err(CypherError::Unsupported(format!(
                    "unsupported binary operator in WHERE expression: {:?}",
                    op
                ))),
            },
            ExprKind::Comparison { left, operators } => {
                if operators.len() != 1 {
                    return Err(CypherError::Unsupported(
                        "chained comparisons are not supported".into(),
                    ));
                }
                let (operator, right) = operators[0];
                let left_expr = self.expression(*left)?;
                let right_expr = self.expression(right)?;
                match operator {
                    ComparisonOperator::Eq => Ok(WhereExpr::Eq(left_expr, right_expr)),
                    ComparisonOperator::Ne => Ok(WhereExpr::NotEq(left_expr, right_expr)),
                    ComparisonOperator::Lt => Ok(WhereExpr::Lt(left_expr, right_expr)),
                    ComparisonOperator::Gt => Ok(WhereExpr::Gt(left_expr, right_expr)),
                    ComparisonOperator::Le => Ok(WhereExpr::Le(left_expr, right_expr)),
                    ComparisonOperator::Ge => Ok(WhereExpr::Ge(left_expr, right_expr)),
                    ComparisonOperator::StartsWith => Ok(WhereExpr::StartsWith(
                        left_expr,
                        self.literal_string(right)?,
                    )),
                    ComparisonOperator::EndsWith => {
                        Ok(WhereExpr::EndsWith(left_expr, self.literal_string(right)?))
                    }
                    ComparisonOperator::Contains => {
                        Ok(WhereExpr::Contains(left_expr, self.literal_string(right)?))
                    }
                    other => Err(CypherError::Unsupported(format!(
                        "unsupported comparison operator in WHERE clause: {other:?}"
                    ))),
                }
            }
            ExprKind::Unary { op, expr } => match op {
                HirUnaryOp::Not => Ok(WhereExpr::Not(Box::new(self.where_expr(*expr)?))),
                _ => Err(CypherError::Unsupported(format!(
                    "unsupported unary operator in WHERE expression: {:?}",
                    op
                ))),
            },
            ExprKind::IsNull { operand, negated } => {
                let expr = self.expression(*operand)?;
                if *negated {
                    Ok(WhereExpr::IsNotNull(expr))
                } else {
                    Ok(WhereExpr::IsNull(expr))
                }
            }
            ExprKind::In { lhs, rhs } => {
                let left_expr = self.expression(*lhs)?;
                let right_expr = self.expression(*rhs)?;
                match right_expr {
                    Expression::List(items) => {
                        let values = items
                            .into_iter()
                            .map(|expr| match expr {
                                Expression::Literal(v) => Ok(v),
                                other => Err(CypherError::Unsupported(format!(
                                    "IN list must contain literals, got: {:?}",
                                    other
                                ))),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(WhereExpr::In(left_expr, values))
                    }
                    Expression::Literal(CypherValue::List(values)) => {
                        Ok(WhereExpr::In(left_expr, values))
                    }
                    other => Err(CypherError::Unsupported(format!(
                        "IN requires a list literal, got: {:?}",
                        other
                    ))),
                }
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
            ExprKind::Literal(literal) => Ok(Expression::Literal(self.literal_to_value(literal)?)),
            ExprKind::Unary { op, expr } => {
                let operand = self.expression(*expr)?;
                let unary_op = match op {
                    HirUnaryOp::Not => UnaryOp::Not,
                    HirUnaryOp::Negate => UnaryOp::Negate,
                    HirUnaryOp::Plus => UnaryOp::Plus,
                };
                Ok(Expression::Unary(unary_op, Box::new(operand)))
            }
            ExprKind::Binary { op, left, right } => {
                let left_expr = self.expression(*left)?;
                let right_expr = self.expression(*right)?;
                let binary_op = match op {
                    HirBinaryOp::Add => BinaryOp::Add,
                    HirBinaryOp::Subtract => BinaryOp::Subtract,
                    HirBinaryOp::Multiply => BinaryOp::Multiply,
                    HirBinaryOp::Divide => BinaryOp::Divide,
                    HirBinaryOp::Modulo => BinaryOp::Modulo,
                    HirBinaryOp::Power => BinaryOp::Power,
                    HirBinaryOp::Eq => BinaryOp::Eq,
                    HirBinaryOp::Ne => BinaryOp::Ne,
                    HirBinaryOp::Lt => BinaryOp::Lt,
                    HirBinaryOp::Gt => BinaryOp::Gt,
                    HirBinaryOp::Le => BinaryOp::Le,
                    HirBinaryOp::Ge => BinaryOp::Ge,
                    HirBinaryOp::And => BinaryOp::And,
                    HirBinaryOp::Or => BinaryOp::Or,
                    HirBinaryOp::Xor => BinaryOp::Xor,
                    HirBinaryOp::StartsWith => BinaryOp::StartsWith,
                    HirBinaryOp::EndsWith => BinaryOp::EndsWith,
                    HirBinaryOp::Contains => BinaryOp::Contains,
                    HirBinaryOp::In => BinaryOp::In,
                    _ => {
                        return Err(CypherError::Unsupported(format!(
                            "unsupported binary operator in expression: {:?}",
                            op
                        )))
                    }
                };
                Ok(Expression::Binary(
                    binary_op,
                    Box::new(left_expr),
                    Box::new(right_expr),
                ))
            }
            ExprKind::List(items) => {
                let expressions = items
                    .iter()
                    .map(|&id| self.expression(id))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Expression::List(expressions))
            }
            ExprKind::Map(entries) => {
                let mut map = HashMap::new();
                for (key_id, value_id) in entries {
                    let key = self.property_key(*key_id)?.to_string();
                    let value = self.expression(*value_id)?;
                    match value {
                        Expression::Literal(v) => {
                            map.insert(key, v);
                        }
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "map values must be literals, got: {:?}",
                                other
                            )));
                        }
                    }
                }
                Ok(Expression::Literal(CypherValue::Map(map)))
            }
            ExprKind::FunctionCall {
                function,
                args,
                distinct,
            } => {
                let name = self.function_name(*function)?.to_string();
                let args = args
                    .iter()
                    .map(|&arg| self.expression(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Expression::FunctionCall {
                    name,
                    args,
                    distinct: *distinct,
                })
            }
            ExprKind::Parameter(param_id) => {
                let name = self
                    .query
                    .arenas
                    .parameters
                    .name_of(*param_id)
                    .ok_or_else(|| {
                        CypherError::InvalidQuery(format!(
                            "unknown parameter id in HIR: {:?}",
                            param_id
                        ))
                    })?;
                Ok(Expression::Parameter(name.to_string()))
            }
            ExprKind::Case(case) => {
                let scrutinee = case
                    .scrutinee
                    .map(|id| self.expression(id))
                    .transpose()?
                    .map(Box::new);
                let mut alternatives = Vec::new();
                for alt in &case.alternatives {
                    let when = self.expression(alt.when)?;
                    let then = self.expression(alt.then)?;
                    alternatives.push((when, then));
                }
                let default = case
                    .default
                    .map(|id| self.expression(id))
                    .transpose()?
                    .map(Box::new);
                Ok(Expression::Case(CaseExpr {
                    scrutinee,
                    alternatives,
                    default,
                }))
            }
            ExprKind::CountStar => Ok(Expression::FunctionCall {
                name: "count".to_string(),
                args: vec![Expression::All],
                distinct: false,
            }),
            other => Err(CypherError::Unsupported(format!(
                "unsupported expression in query planner: {other:?}"
            ))),
        }
    }

    fn literal_value(&self, expr_id: ExprId) -> Result<CypherValue, CypherError> {
        match &self.expr(expr_id)?.kind {
            ExprKind::Literal(literal) => self.literal_to_value(literal),
            other => Err(CypherError::Unsupported(format!(
                "unsupported literal expression: {other:?}"
            ))),
        }
    }

    fn literal_string(&self, expr_id: ExprId) -> Result<String, CypherError> {
        match self.literal_value(expr_id)? {
            CypherValue::String(s) => Ok(s),
            other => Err(CypherError::Unsupported(format!(
                "expected string literal, got: {:?}",
                other
            ))),
        }
    }

    fn literal_to_value(&self, literal: &Literal) -> Result<CypherValue, CypherError> {
        Ok(match literal {
            Literal::Null => CypherValue::Null,
            Literal::Boolean(value) => CypherValue::Boolean(*value),
            Literal::Integer(value) => CypherValue::Integer(*value),
            Literal::Float(value) => CypherValue::Float(*value),
            Literal::String(value) => CypherValue::String(value.clone()),
        })
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

    fn set_items(
        &self,
        items: &[decypher::hir::ops::SetItem],
    ) -> Result<Vec<SetItem>, CypherError> {
        items
            .iter()
            .map(|item| match item {
                decypher::hir::ops::SetItem::SetProperty { target, value } => {
                    let target_expr = self.expression(*target)?;
                    let (variable, property) = match target_expr {
                        Expression::Property(var, prop) => (var, prop),
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "SET property target must be a property access, got: {:?}",
                                other
                            )));
                        }
                    };
                    let value_expr = self.expression(*value)?;
                    Ok(SetItem::SetProperty {
                        variable,
                        property,
                        value: value_expr,
                    })
                }
                decypher::hir::ops::SetItem::SetVariable { target, value } => {
                    let variable = self.binding_name(*target)?.to_string();
                    let properties = match self.expression(*value)? {
                        Expression::Literal(CypherValue::Map(map)) => map,
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "SET variable value must be a map literal, got: {:?}",
                                other
                            )));
                        }
                    };
                    Ok(SetItem::SetVariable {
                        variable,
                        properties,
                    })
                }
                decypher::hir::ops::SetItem::SetLabels { node, labels } => {
                    let variable = self.binding_name(*node)?.to_string();
                    let labels = labels
                        .iter()
                        .map(|&l| self.label_name(l).map(str::to_string))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(SetItem::SetLabels { variable, labels })
                }
                decypher::hir::ops::SetItem::ReplaceProperties { entity, value } => {
                    let variable = self.binding_name(*entity)?.to_string();
                    let properties = match self.expression(*value)? {
                        Expression::Literal(CypherValue::Map(map)) => map,
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "SET properties value must be a map literal, got: {:?}",
                                other
                            )));
                        }
                    };
                    Ok(SetItem::SetVariable {
                        variable,
                        properties,
                    })
                }
                decypher::hir::ops::SetItem::MergeProperties { entity, value } => {
                    let variable = self.binding_name(*entity)?.to_string();
                    let properties = match self.expression(*value)? {
                        Expression::Literal(CypherValue::Map(map)) => map,
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "SET merge properties value must be a map literal, got: {:?}",
                                other
                            )));
                        }
                    };
                    Ok(SetItem::MergeProperties {
                        variable,
                        properties,
                    })
                }
            })
            .collect()
    }

    fn remove_items(
        &self,
        items: &[decypher::hir::ops::RemoveItem],
    ) -> Result<Vec<RemoveItem>, CypherError> {
        items
            .iter()
            .map(|item| match item {
                decypher::hir::ops::RemoveItem::Property { target } => {
                    let target_expr = self.expression(*target)?;
                    let (variable, property) = match target_expr {
                        Expression::Property(var, prop) => (var, prop),
                        other => {
                            return Err(CypherError::Unsupported(format!(
                                "REMOVE property target must be a property access, got: {:?}",
                                other
                            )));
                        }
                    };
                    Ok(RemoveItem::RemoveProperty { variable, property })
                }
                decypher::hir::ops::RemoveItem::Labels { node, labels } => {
                    let variable = self.binding_name(*node)?.to_string();
                    let labels = labels
                        .iter()
                        .map(|&l| self.label_name(l).map(str::to_string))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(RemoveItem::RemoveLabels { variable, labels })
                }
            })
            .collect()
    }

    fn usize_from_expr(&self, expr_id: ExprId) -> Result<usize, CypherError> {
        match self.literal_value(expr_id)? {
            CypherValue::Integer(v) if v >= 0 => Ok(v as usize),
            other => Err(CypherError::InvalidQuery(format!(
                "expected non-negative integer, got: {:?}",
                other
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
        let length = match relationship.length {
            decypher::hir::pattern::RelationshipLength::Single => None,
            decypher::hir::pattern::RelationshipLength::Variable { min, max } => {
                Some(RelationshipLength {
                    min: min.map(|v| v as usize),
                    max: max.map(|v| v as usize),
                })
            }
        };

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
            length,
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

    fn label_name(&self, label: decypher::hir::arena::LabelId) -> Result<&str, CypherError> {
        self.query.arenas.labels.name_of(label).ok_or_else(|| {
            CypherError::InvalidQuery(format!("unknown label id in HIR: {:?}", label))
        })
    }

    fn rel_type_name(
        &self,
        rel_type: decypher::hir::arena::RelTypeId,
    ) -> Result<&str, CypherError> {
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
        property: decypher::hir::arena::PropertyKeyId,
    ) -> Result<&str, CypherError> {
        self.query
            .arenas
            .property_keys
            .name_of(property)
            .ok_or_else(|| {
                CypherError::InvalidQuery(format!("unknown property key id in HIR: {:?}", property))
            })
    }

    fn function_name(
        &self,
        function: decypher::hir::arena::FunctionId,
    ) -> Result<&str, CypherError> {
        self.query
            .arenas
            .functions
            .name_of(function)
            .ok_or_else(|| {
                CypherError::InvalidQuery(format!("unknown function id in HIR: {:?}", function))
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

    #[test]
    fn plan_match_with_trailing_semicolon() {
        // A MATCH query ending with `;` must not become `MATCH (n); RETURN *`
        // (two statements). Trailing semicolons are stripped before the retry.
        let q = plan_query("MATCH (n);").expect("MATCH with trailing semicolon should work");
        assert_eq!(q.clauses.len(), 1);
        assert!(matches!(q.clauses[0], Clause::Match { .. }));
    }
}
