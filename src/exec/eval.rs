//! Expression evaluation directly over the HIR expression arena.

use std::cmp::Ordering;

use decypher::hir::expr::{
    BinaryOp, CaseExpr, CollectionQuantifier, ComparisonOperator, Literal, MapProjectionItem,
    UnaryOp,
};
use decypher::hir::{ExprId, ExprKind};

use super::functions::{self, Function, Scalar, format_float};
use super::{Access, Executor, Result, Row};
use crate::element::{CypherEdge, CypherNode};
use crate::error::CypherError;
use crate::graph::CypherGraph;
use crate::value::{Properties, Value};

fn type_error(what: &str, value: &Value) -> CypherError {
    CypherError::TypeMismatch(format!("{what} does not accept {}", value.type_name()))
}

fn overflow() -> CypherError {
    CypherError::RuntimeError("integer overflow".into())
}

/// Three-valued logic: `Some(bool)` or `None` for `null`.
fn truth(value: &Value, what: &str) -> Result<Option<bool>> {
    match value {
        Value::Boolean(b) => Ok(Some(*b)),
        Value::Null => Ok(None),
        other => Err(type_error(what, other)),
    }
}

fn from_truth(t: Option<bool>) -> Value {
    t.map_or(Value::Null, Value::Boolean)
}

/// `<`, `<=`, `>`, `>=`, `=`, `<>` with Cypher's `null` semantics.
fn compare(op: ComparisonOperator, a: &Value, b: &Value) -> Result<Value> {
    use ComparisonOperator as C;
    Ok(match op {
        C::Eq => from_truth(a.cypher_eq(b)),
        C::Ne => from_truth(a.cypher_eq(b).map(|e| !e)),
        C::Lt | C::Le | C::Gt | C::Ge => {
            if a.is_null() || b.is_null() {
                return Ok(Value::Null);
            }
            let nan = |v: &Value| matches!(v, Value::Float(f) if f.is_nan());
            if (nan(a) || nan(b)) && a.as_f64().is_some() && b.as_f64().is_some() {
                return Ok(Value::Boolean(false));
            }
            match a.cypher_cmp(b) {
                None => Value::Null,
                Some(o) => Value::Boolean(match op {
                    C::Lt => o == Ordering::Less,
                    C::Le => o != Ordering::Greater,
                    C::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                }),
            }
        }
        C::StartsWith | C::EndsWith | C::Contains => match (a, b) {
            (Value::String(s), Value::String(t)) => Value::Boolean(match op {
                C::StartsWith => s.starts_with(t.as_str()),
                C::EndsWith => s.ends_with(t.as_str()),
                _ => s.contains(t.as_str()),
            }),
            _ => Value::Null,
        },
        C::RegexMatch => {
            return Err(CypherError::Unsupported("regular expressions (=~)".into()));
        }
    })
}

fn comparison_op(op: BinaryOp) -> Option<ComparisonOperator> {
    use ComparisonOperator as C;
    Some(match op {
        BinaryOp::Eq => C::Eq,
        BinaryOp::Ne => C::Ne,
        BinaryOp::Lt => C::Lt,
        BinaryOp::Le => C::Le,
        BinaryOp::Gt => C::Gt,
        BinaryOp::Ge => C::Ge,
        BinaryOp::RegexMatch => C::RegexMatch,
        BinaryOp::StartsWith => C::StartsWith,
        BinaryOp::EndsWith => C::EndsWith,
        BinaryOp::Contains => C::Contains,
        _ => return None,
    })
}

fn arithmetic(op: BinaryOp, a: Value, b: Value) -> Result<Value> {
    use Value as V;
    if a.is_null() || b.is_null() {
        return Ok(V::Null);
    }
    match op {
        BinaryOp::Add => match (a, b) {
            (V::Integer(x), V::Integer(y)) => x.checked_add(y).map(V::Integer).ok_or_else(overflow),
            (V::List(mut x), V::List(y)) => {
                x.extend(y);
                Ok(V::List(x))
            }
            (V::List(mut x), y) => {
                x.push(y);
                Ok(V::List(x))
            }
            (x, V::List(mut y)) => {
                y.insert(0, x);
                Ok(V::List(y))
            }
            (V::String(x), y) => Ok(V::String(x + &concat_part(&y)?)),
            (x, V::String(y)) => Ok(V::String(concat_part(&x)? + &y)),
            (x, y) => float_op(&x, &y, "+", |p, q| p + q),
        },
        BinaryOp::Subtract => match (&a, &b) {
            (V::Integer(x), V::Integer(y)) => {
                x.checked_sub(*y).map(V::Integer).ok_or_else(overflow)
            }
            _ => float_op(&a, &b, "-", |p, q| p - q),
        },
        BinaryOp::Multiply => match (&a, &b) {
            (V::Integer(x), V::Integer(y)) => {
                x.checked_mul(*y).map(V::Integer).ok_or_else(overflow)
            }
            _ => float_op(&a, &b, "*", |p, q| p * q),
        },
        BinaryOp::Divide => match (&a, &b) {
            (V::Integer(_), V::Integer(0)) => Err(CypherError::DivisionByZero),
            (V::Integer(x), V::Integer(y)) => {
                x.checked_div(*y).map(V::Integer).ok_or_else(overflow)
            }
            _ => float_op(&a, &b, "/", |p, q| p / q),
        },
        BinaryOp::Modulo => match (&a, &b) {
            (V::Integer(_), V::Integer(0)) => Err(CypherError::DivisionByZero),
            (V::Integer(x), V::Integer(y)) => {
                x.checked_rem(*y).map(V::Integer).ok_or_else(overflow)
            }
            _ => float_op(&a, &b, "%", |p, q| p % q),
        },
        // `^` always yields a float.
        BinaryOp::Power => float_op(&a, &b, "^", f64::powf),
        other => Err(CypherError::Unsupported(format!("operator {other:?}"))),
    }
}

fn concat_part(v: &Value) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Float(f) => Ok(format_float(*f)),
        Value::Boolean(b) => Ok(b.to_string()),
        other => Err(type_error("+", other)),
    }
}

fn float_op(a: &Value, b: &Value, op: &str, f: impl Fn(f64, f64) -> f64) -> Result<Value> {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => Ok(Value::Float(f(x, y))),
        (None, _) => Err(type_error(op, a)),
        _ => Err(type_error(op, b)),
    }
}

/// A list index, counting from the end when negative.
fn list_index(len: usize, i: i64) -> Option<usize> {
    let i = if i < 0 { len as i64 + i } else { i };
    (0..len as i64).contains(&i).then_some(i as usize)
}

impl<A: Access> Executor<'_, A> {
    pub(super) fn eval(&self, id: ExprId, row: &Row) -> Result<Value> {
        let q = self.q;
        let expr = &q.hir.arenas.expressions.get(id).kind;
        match expr {
            ExprKind::Literal(lit) => Ok(match lit {
                Literal::Null => Value::Null,
                Literal::Boolean(b) => Value::Boolean(*b),
                Literal::Integer(i) => Value::Integer(*i),
                Literal::Float(f) => Value::Float(*f),
                Literal::String(s) => Value::String(s.clone()),
            }),
            ExprKind::Binding(b) => Ok(row.get(b.0).and_then(Clone::clone).unwrap_or_default()),
            ExprKind::Parameter(p) => Ok(self.params[p.0].clone()),
            ExprKind::Property { base, key } => {
                let base = self.eval(*base, row)?;
                self.property(&base, &q.keys[key.0])
            }
            ExprKind::List(items) => Ok(Value::List(
                items
                    .iter()
                    .map(|&e| self.eval(e, row))
                    .collect::<Result<_>>()?,
            )),
            ExprKind::Map(entries) => Ok(Value::Map(
                entries
                    .iter()
                    .map(|(k, e)| Ok((q.keys[k.0].clone(), self.eval(*e, row)?)))
                    .collect::<Result<_>>()?,
            )),
            ExprKind::Unary { op, expr } => {
                let v = self.eval(*expr, row)?;
                match (op, v) {
                    (_, Value::Null) => Ok(Value::Null),
                    (UnaryOp::Not, v) => Ok(from_truth(truth(&v, "NOT")?.map(|b| !b))),
                    (UnaryOp::Plus, v @ (Value::Integer(_) | Value::Float(_))) => Ok(v),
                    (UnaryOp::Negate, Value::Integer(i)) => {
                        i.checked_neg().map(Value::Integer).ok_or_else(overflow)
                    }
                    (UnaryOp::Negate, Value::Float(f)) => Ok(Value::Float(-f)),
                    (_, v) => Err(type_error("unary operator", &v)),
                }
            }
            ExprKind::Binary { op, left, right } => self.binary(*op, *left, *right, row),
            ExprKind::Comparison { left, operators } => self.comparison(*left, operators, row),
            ExprKind::In { lhs, rhs } => {
                let needle = self.eval(*lhs, row)?;
                self.contains(&needle, self.eval(*rhs, row)?)
            }
            ExprKind::IsNull { operand, negated } => {
                let null = self.eval(*operand, row)?.is_null();
                Ok(Value::Boolean(null != *negated))
            }
            ExprKind::FunctionCall { function, args, .. } => {
                self.call(q.functions[function.0], args, row)
            }
            ExprKind::CountStar => Err(CypherError::InvalidQuery(
                "count(*) outside of an aggregation".into(),
            )),
            ExprKind::Case(case) => self.case(case, row),
            ExprKind::ListComprehension(lc) => {
                let items = match self.eval(lc.collection, row)? {
                    Value::Null => return Ok(Value::Null),
                    Value::List(items) => items,
                    other => return Err(type_error("list comprehension", &other)),
                };
                let mut inner = row.clone();
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    inner[lc.variable.0] = Some(item);
                    if let Some(filter) = lc.filter
                        && !self.is_true(filter, &inner)?
                    {
                        continue;
                    }
                    out.push(match lc.map {
                        Some(map) => self.eval(map, &inner)?,
                        None => inner[lc.variable.0].clone().unwrap_or_default(),
                    });
                }
                Ok(Value::List(out))
            }
            ExprKind::PatternComprehension(pc) => {
                let matches = self.match_path(&pc.pattern, pc.variable, row)?;
                let mut out = Vec::with_capacity(matches.len());
                for m in matches {
                    if let Some(filter) = pc.filter
                        && !self.is_true(filter, &m)?
                    {
                        continue;
                    }
                    out.push(self.eval(pc.map, &m)?);
                }
                Ok(Value::List(out))
            }
            ExprKind::PatternExpr(pattern) => Ok(Value::Boolean(
                !self.match_pattern(pattern, row, &[], Some(1))?.is_empty(),
            )),
            ExprKind::CollectionFilter {
                quantifier,
                variable,
                collection,
                predicate,
            } => {
                let items = match self.eval(*collection, row)? {
                    Value::Null => return Ok(Value::Null),
                    Value::List(items) => items,
                    other => return Err(type_error("list predicate", &other)),
                };
                let mut inner = row.clone();
                let (mut trues, mut nulls) = (0usize, 0usize);
                let total = items.len();
                for item in items {
                    inner[variable.0] = Some(item);
                    let t = match predicate {
                        Some(p) => truth(&self.eval(*p, &inner)?, "list predicate")?,
                        None => Some(true),
                    };
                    match t {
                        Some(true) => trues += 1,
                        None => nulls += 1,
                        Some(false) => {}
                    }
                }
                let falses = total - trues - nulls;
                Ok(from_truth(match quantifier {
                    CollectionQuantifier::All if falses > 0 => Some(false),
                    CollectionQuantifier::All => (nulls == 0).then_some(true),
                    CollectionQuantifier::Any if trues > 0 => Some(true),
                    CollectionQuantifier::Any => (nulls == 0).then_some(false),
                    CollectionQuantifier::None if trues > 0 => Some(false),
                    CollectionQuantifier::None => (nulls == 0).then_some(true),
                    CollectionQuantifier::Single if trues > 1 => Some(false),
                    CollectionQuantifier::Single if nulls > 0 => None,
                    CollectionQuantifier::Single => Some(trues == 1),
                }))
            }
            ExprKind::Reduce {
                accumulator,
                init,
                variable,
                list,
                expression,
            } => {
                let items = match self.eval(*list, row)? {
                    Value::Null => return Ok(Value::Null),
                    Value::List(items) => items,
                    other => return Err(type_error("reduce", &other)),
                };
                let mut inner = row.clone();
                inner[accumulator.0] = Some(self.eval(*init, row)?);
                for item in items {
                    inner[variable.0] = Some(item);
                    let next = self.eval(*expression, &inner)?;
                    inner[accumulator.0] = Some(next);
                }
                Ok(inner[accumulator.0].take().unwrap_or_default())
            }
            ExprKind::NodeLabels { base, labels } => match self.eval(*base, row)? {
                Value::Null => Ok(Value::Null),
                Value::Node(n) => {
                    let node = self.graph().node(n);
                    Ok(Value::Boolean(node.is_some_and(|w| {
                        labels.iter().all(|l| w.has_label(&q.labels[l.0]))
                    })))
                }
                other => Err(type_error("label predicate", &other)),
            },
            ExprKind::LabelCheck { base, expression } => match self.eval(*base, row)? {
                Value::Null => Ok(Value::Null),
                Value::Node(n) => {
                    let Some(w) = self.graph().node(n) else {
                        return Ok(Value::Boolean(false));
                    };
                    let any = w.labels().next().is_some();
                    Ok(Value::Boolean(self.label_expr_matches(
                        expression,
                        &|l| w.has_label(l),
                        any,
                        false,
                        row,
                    )?))
                }
                Value::Relationship(e) => {
                    let Some(w) = self.graph().edge(e) else {
                        return Ok(Value::Boolean(false));
                    };
                    let ty = w.rel_type();
                    Ok(Value::Boolean(self.label_expr_matches(
                        expression,
                        &|t| t == ty,
                        true,
                        true,
                        row,
                    )?))
                }
                other => Err(type_error("label predicate", &other)),
            },
            ExprKind::ListIndex { list, index } => {
                let container = self.eval(*list, row)?;
                let index = self.eval(*index, row)?;
                match (container, index) {
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    (Value::List(items), Value::Integer(i)) => Ok(list_index(items.len(), i)
                        .map(|i| items[i].clone())
                        .unwrap_or_default()),
                    (Value::List(_), other) => Err(type_error("list index", &other)),
                    (
                        base @ (Value::Map(_) | Value::Node(_) | Value::Relationship(_)),
                        Value::String(k),
                    ) => self.property(&base, &k),
                    (base, _) => Err(type_error("index access", &base)),
                }
            }
            ExprKind::ListSlice { list, start, end } => {
                let items = match self.eval(*list, row)? {
                    Value::Null => return Ok(Value::Null),
                    Value::List(items) => items,
                    other => return Err(type_error("list slice", &other)),
                };
                let len = items.len() as i64;
                let bound = |e: &Option<ExprId>, default: i64| -> Result<Option<i64>> {
                    match e {
                        None => Ok(Some(default)),
                        Some(e) => match self.eval(*e, row)? {
                            Value::Null => Ok(None),
                            Value::Integer(i) => {
                                Ok(Some(if i < 0 { (len + i).max(0) } else { i.min(len) }))
                            }
                            other => Err(type_error("list slice", &other)),
                        },
                    }
                };
                let (Some(from), Some(to)) = (bound(start, 0)?, bound(end, len)?) else {
                    return Ok(Value::Null);
                };
                Ok(Value::List(if from < to {
                    items[from as usize..to as usize].to_vec()
                } else {
                    Vec::new()
                }))
            }
            ExprKind::MapProjection { base, items } => {
                let base = row[base.0].clone().unwrap_or_default();
                if base.is_null() {
                    return Ok(Value::Null);
                }
                let mut map = Properties::new();
                for item in items {
                    match item {
                        MapProjectionItem::AllProperties => {
                            map.extend(self.element_properties(&base).unwrap_or_default());
                        }
                        MapProjectionItem::PropertyLookup { key } => {
                            let k = &q.keys[key.0];
                            map.insert(k.clone(), self.property(&base, k)?);
                        }
                        MapProjectionItem::Literal { key, value } => {
                            map.insert(q.keys[key.0].clone(), self.eval(*value, row)?);
                        }
                    }
                }
                Ok(Value::Map(map))
            }
            ExprKind::ExistsSubquery(_)
            | ExprKind::CountSubquery(_)
            | ExprKind::CollectSubquery(_) => Err(CypherError::Unsupported(
                "EXISTS / COUNT / COLLECT subqueries".into(),
            )),
            ExprKind::IsNormalized { .. } => Err(CypherError::Unsupported("IS NORMALIZED".into())),
            ExprKind::IsTyped { .. } => Err(CypherError::Unsupported("type predicates".into())),
        }
    }

    /// A comparison chain `a < b < c`: `a < b AND b < c`, each operand
    /// evaluated once.
    fn comparison(
        &self,
        left: ExprId,
        operators: &[(ComparisonOperator, ExprId)],
        row: &Row,
    ) -> Result<Value> {
        let mut lhs = self.eval(left, row)?;
        let mut result = Some(true);
        for (op, rhs) in operators {
            let rhs = self.eval(*rhs, row)?;
            match truth(&compare(*op, &lhs, &rhs)?, "comparison")? {
                Some(false) => result = Some(false),
                None if result != Some(false) => result = None,
                _ => {}
            }
            lhs = rhs;
        }
        Ok(from_truth(result))
    }

    fn binary(&self, op: BinaryOp, left: ExprId, right: ExprId, row: &Row) -> Result<Value> {
        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                let a = truth(&self.eval(left, row)?, "boolean operator")?;
                let b = truth(&self.eval(right, row)?, "boolean operator")?;
                Ok(from_truth(match (op, a, b) {
                    (BinaryOp::And, Some(false), _) | (BinaryOp::And, _, Some(false)) => {
                        Some(false)
                    }
                    (BinaryOp::And, Some(true), Some(true)) => Some(true),
                    (BinaryOp::Or, Some(true), _) | (BinaryOp::Or, _, Some(true)) => Some(true),
                    (BinaryOp::Or, Some(false), Some(false)) => Some(false),
                    (BinaryOp::Xor, Some(x), Some(y)) => Some(x != y),
                    _ => None,
                }))
            }
            BinaryOp::In => {
                let needle = self.eval(left, row)?;
                self.contains(&needle, self.eval(right, row)?)
            }
            BinaryOp::Is | BinaryOp::IsNot => {
                Err(CypherError::Unsupported(format!("operator {op:?}")))
            }
            _ => {
                let a = self.eval(left, row)?;
                let b = self.eval(right, row)?;
                match comparison_op(op) {
                    Some(c) => compare(c, &a, &b),
                    None => arithmetic(op, a, b),
                }
            }
        }
    }

    /// `needle IN list`.
    fn contains(&self, needle: &Value, list: Value) -> Result<Value> {
        let items = match list {
            Value::Null => return Ok(Value::Null),
            Value::List(items) => items,
            other => return Err(type_error("IN", &other)),
        };
        let mut unknown = false;
        for item in &items {
            match needle.cypher_eq(item) {
                Some(true) => return Ok(Value::Boolean(true)),
                None => unknown = true,
                Some(false) => {}
            }
        }
        Ok(if unknown {
            Value::Null
        } else {
            Value::Boolean(false)
        })
    }

    fn case(&self, case: &CaseExpr, row: &Row) -> Result<Value> {
        let scrutinee = case.scrutinee.map(|s| self.eval(s, row)).transpose()?;
        for alt in &case.alternatives {
            let when = self.eval(alt.when, row)?;
            let hit = match &scrutinee {
                Some(s) => s.cypher_eq(&when) == Some(true),
                None => truth(&when, "CASE WHEN")? == Some(true),
            };
            if hit {
                return self.eval(alt.then, row);
            }
        }
        case.default.map_or(Ok(Value::Null), |d| self.eval(d, row))
    }

    /// `base.key` on a node, relationship or map.
    fn property(&self, base: &Value, key: &str) -> Result<Value> {
        self.check_alive(base)?;
        let graph = self.graph();
        Ok(match base {
            Value::Null => Value::Null,
            Value::Node(n) => graph
                .node(*n)
                .and_then(|w| w.property(key))
                .cloned()
                .unwrap_or_default(),
            Value::Relationship(e) => graph
                .edge(*e)
                .and_then(|w| w.property(key))
                .cloned()
                .unwrap_or_default(),
            Value::Map(map) => map.get(key).cloned().unwrap_or_default(),
            other => return Err(type_error("property access", other)),
        })
    }

    /// Reading a node or relationship deleted earlier in the query is an
    /// error.
    fn check_alive(&self, value: &Value) -> Result<()> {
        let deleted = match value {
            Value::Node(n) => !self.node_alive(*n),
            Value::Relationship(e) => !self.edge_alive(*e),
            _ => false,
        };
        if deleted {
            Err(CypherError::RuntimeError(format!(
                "{} was deleted earlier in the query",
                value.type_name().to_lowercase()
            )))
        } else {
            Ok(())
        }
    }

    /// The properties of a node or relationship, or a map itself.
    pub(super) fn element_properties(&self, value: &Value) -> Option<Properties> {
        let graph = self.graph();
        let collect = |it: &mut dyn Iterator<Item = (&str, &Value)>| {
            it.map(|(k, v)| (k.to_owned(), v.clone())).collect()
        };
        match value {
            Value::Node(n) => graph.node(*n).map(|w| collect(&mut w.properties())),
            Value::Relationship(e) => graph.edge(*e).map(|w| collect(&mut w.properties())),
            Value::Map(m) => Some(m.clone()),
            _ => None,
        }
    }

    fn call(&self, f: Function, args: &[ExprId], row: &Row) -> Result<Value> {
        let Function::Scalar(f) = f else {
            return Err(CypherError::InvalidQuery(
                "aggregate function in a non-aggregating context".into(),
            ));
        };
        if f == Scalar::Exists {
            let [arg] = args else {
                return Err(CypherError::InvalidQuery(
                    "exists() expects 1 argument".into(),
                ));
            };
            return Ok(match &self.q.hir.arenas.expressions.get(*arg).kind {
                ExprKind::PatternExpr(_) => self.eval(*arg, row)?,
                _ => Value::Boolean(!self.eval(*arg, row)?.is_null()),
            });
        }
        let args = args
            .iter()
            .map(|&a| self.eval(a, row))
            .collect::<Result<Vec<_>>>()?;
        match f {
            Scalar::Id
            | Scalar::Labels
            | Scalar::Type
            | Scalar::Properties
            | Scalar::Keys
            | Scalar::StartNode
            | Scalar::EndNode
            | Scalar::Nodes
            | Scalar::Relationships
            | Scalar::Length => self.graph_function(f, args),
            _ => functions::call(f, args),
        }
    }

    fn graph_function(&self, f: Scalar, args: Vec<Value>) -> Result<Value> {
        let [arg] = <[Value; 1]>::try_from(args)
            .map_err(|_| CypherError::InvalidQuery(format!("{f:?}() expects 1 argument")))?;
        // Labels and properties of deleted elements are gone; their identity
        // and type are not.
        if matches!(f, Scalar::Labels | Scalar::Properties | Scalar::Keys) {
            self.check_alive(&arg)?;
        }
        let graph = self.graph();
        Ok(match (f, arg) {
            (_, Value::Null) => Value::Null,
            (Scalar::Id, Value::Node(n)) => Value::Integer(n.index() as i64),
            (Scalar::Id, Value::Relationship(e)) => Value::Integer(e.index() as i64),
            (Scalar::Labels, Value::Node(n)) => Value::List(
                graph
                    .node(n)
                    .map(|w| w.labels().map(Value::from).collect())
                    .unwrap_or_default(),
            ),
            (Scalar::Type, Value::Relationship(e)) => graph
                .edge(e)
                .map_or(Value::Null, |w| Value::from(w.rel_type())),
            (Scalar::Properties, v @ (Value::Node(_) | Value::Relationship(_) | Value::Map(_))) => {
                self.element_properties(&v).map_or(Value::Null, Value::Map)
            }
            (Scalar::Keys, Value::Node(n)) => Value::List(
                graph
                    .node(n)
                    .map(|w| w.properties().map(|(k, _)| Value::from(k)).collect())
                    .unwrap_or_default(),
            ),
            (Scalar::Keys, Value::Relationship(e)) => Value::List(
                graph
                    .edge(e)
                    .map(|w| w.properties().map(|(k, _)| Value::from(k)).collect())
                    .unwrap_or_default(),
            ),
            (Scalar::Keys, Value::Map(m)) => Value::List(m.into_keys().map(Value::from).collect()),
            (Scalar::StartNode | Scalar::EndNode, Value::Relationship(e)) => {
                graph.endpoints(e).map_or(Value::Null, |(s, t)| {
                    Value::Node(if f == Scalar::StartNode { s } else { t })
                })
            }
            (Scalar::Nodes, Value::Path(p)) => {
                Value::List(p.nodes.into_iter().map(Value::Node).collect())
            }
            (Scalar::Relationships, Value::Path(p)) => Value::List(
                p.relationships
                    .into_iter()
                    .map(Value::Relationship)
                    .collect(),
            ),
            (Scalar::Length, Value::Path(p)) => Value::Integer(p.relationships.len() as i64),
            (Scalar::Length, v @ (Value::String(_) | Value::List(_))) => {
                functions::call(Scalar::Size, vec![v])?
            }
            (f, v) => return Err(type_error(&format!("{f:?}()"), &v)),
        })
    }
}
