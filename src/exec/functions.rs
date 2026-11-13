//! Scalar and aggregate functions, resolved from their name once when a query
//! is prepared.

use std::collections::BTreeSet;

use super::Ordered;
use crate::error::CypherError;
use crate::value::Value;

/// A function the engine implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Function {
    Scalar(Scalar),
    Aggregate(Aggregate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Aggregate {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Collect,
    StDev,
    StDevP,
    Var,
    VarP,
    PercentileCont,
    PercentileDisc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scalar {
    // Graph elements and paths, evaluated by the executor.
    Exists,
    Id,
    Labels,
    Type,
    Properties,
    Keys,
    StartNode,
    EndNode,
    Nodes,
    Relationships,
    Length,
    // Values.
    Coalesce,
    Size,
    Head,
    Last,
    Tail,
    Range,
    Reverse,
    ToBoolean,
    ToInteger,
    ToFloat,
    ToString,
    ToUpper,
    ToLower,
    Trim,
    LTrim,
    RTrim,
    Substring,
    Left,
    Right,
    Replace,
    Split,
    Abs,
    Ceil,
    Floor,
    Round,
    Sign,
    Sqrt,
    Exp,
    Log,
    Log10,
    E,
    Pi,
    Sin,
    Cos,
    Tan,
    Cot,
    Asin,
    Acos,
    Atan,
    Atan2,
    Degrees,
    Radians,
    Haversin,
    Rand,
    ToBooleanOrNull,
    ToIntegerOrNull,
    ToFloatOrNull,
    ToStringOrNull,
    ToBooleanList,
    ToIntegerList,
    ToFloatList,
    ToStringList,
    IsEmpty,
    IsNaN,
    CharLength,
    NullIf,
    BTrim,
    ElementId,
    RandomUuid,
    ValueType,
    // Evaluated by the executor: fixed for the whole query.
    Timestamp,
}

/// Scalar functions by their Cypher name.
const SCALARS: &[(&str, Scalar)] = &[
    ("exists", Scalar::Exists),
    ("id", Scalar::Id),
    ("labels", Scalar::Labels),
    ("type", Scalar::Type),
    ("properties", Scalar::Properties),
    ("keys", Scalar::Keys),
    ("startNode", Scalar::StartNode),
    ("endNode", Scalar::EndNode),
    ("nodes", Scalar::Nodes),
    ("relationships", Scalar::Relationships),
    ("length", Scalar::Length),
    ("coalesce", Scalar::Coalesce),
    ("size", Scalar::Size),
    ("head", Scalar::Head),
    ("last", Scalar::Last),
    ("tail", Scalar::Tail),
    ("range", Scalar::Range),
    ("reverse", Scalar::Reverse),
    ("toBoolean", Scalar::ToBoolean),
    ("toInteger", Scalar::ToInteger),
    ("toFloat", Scalar::ToFloat),
    ("toString", Scalar::ToString),
    ("toUpper", Scalar::ToUpper),
    ("toLower", Scalar::ToLower),
    ("trim", Scalar::Trim),
    ("lTrim", Scalar::LTrim),
    ("rTrim", Scalar::RTrim),
    ("substring", Scalar::Substring),
    ("left", Scalar::Left),
    ("right", Scalar::Right),
    ("replace", Scalar::Replace),
    ("split", Scalar::Split),
    ("abs", Scalar::Abs),
    ("ceil", Scalar::Ceil),
    ("floor", Scalar::Floor),
    ("round", Scalar::Round),
    ("sign", Scalar::Sign),
    ("sqrt", Scalar::Sqrt),
    ("exp", Scalar::Exp),
    ("log", Scalar::Log),
    ("log10", Scalar::Log10),
    ("e", Scalar::E),
    ("pi", Scalar::Pi),
    ("sin", Scalar::Sin),
    ("cos", Scalar::Cos),
    ("tan", Scalar::Tan),
    ("cot", Scalar::Cot),
    ("asin", Scalar::Asin),
    ("acos", Scalar::Acos),
    ("atan", Scalar::Atan),
    ("atan2", Scalar::Atan2),
    ("degrees", Scalar::Degrees),
    ("radians", Scalar::Radians),
    ("haversin", Scalar::Haversin),
    ("rand", Scalar::Rand),
    ("toInt", Scalar::ToInteger),
    ("toBooleanOrNull", Scalar::ToBooleanOrNull),
    ("toIntegerOrNull", Scalar::ToIntegerOrNull),
    ("toFloatOrNull", Scalar::ToFloatOrNull),
    ("toStringOrNull", Scalar::ToStringOrNull),
    ("toBooleanList", Scalar::ToBooleanList),
    ("toIntegerList", Scalar::ToIntegerList),
    ("toFloatList", Scalar::ToFloatList),
    ("toStringList", Scalar::ToStringList),
    ("isEmpty", Scalar::IsEmpty),
    ("isNaN", Scalar::IsNaN),
    ("char_length", Scalar::CharLength),
    ("character_length", Scalar::CharLength),
    ("nullIf", Scalar::NullIf),
    ("lower", Scalar::ToLower),
    ("upper", Scalar::ToUpper),
    ("btrim", Scalar::BTrim),
    ("elementId", Scalar::ElementId),
    ("randomUUID", Scalar::RandomUuid),
    ("valueType", Scalar::ValueType),
    ("timestamp", Scalar::Timestamp),
];

const AGGREGATES: &[(&str, Aggregate)] = &[
    ("count", Aggregate::Count),
    ("sum", Aggregate::Sum),
    ("avg", Aggregate::Avg),
    ("min", Aggregate::Min),
    ("max", Aggregate::Max),
    ("collect", Aggregate::Collect),
    ("stDev", Aggregate::StDev),
    ("stDevP", Aggregate::StDevP),
    ("var", Aggregate::Var),
    ("varP", Aggregate::VarP),
    ("percentileCont", Aggregate::PercentileCont),
    ("percentileDisc", Aggregate::PercentileDisc),
];

fn lookup<T: Copy>(table: &[(&str, T)], name: &str) -> Option<T> {
    table
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, f)| *f)
}

impl Aggregate {
    /// Checks the argument count: `count(*)` has none, percentiles take the
    /// value and the percentile, every other aggregate one value.
    pub(crate) fn check_arity(self, args: usize) -> Result<(), CypherError> {
        let ok = match self {
            Aggregate::Count => args <= 1,
            Aggregate::PercentileCont | Aggregate::PercentileDisc => args == 2,
            _ => args == 1,
        };
        if ok {
            Ok(())
        } else {
            Err(CypherError::InvalidQuery(format!(
                "{self:?}() does not take {args} argument(s)"
            )))
        }
    }

    fn is_percentile(self) -> bool {
        matches!(self, Aggregate::PercentileCont | Aggregate::PercentileDisc)
    }
}

impl Scalar {
    /// The Cypher name, for error messages.
    fn name(self) -> &'static str {
        SCALARS
            .iter()
            .find(|(_, f)| *f == self)
            .map_or("function", |(name, _)| name)
    }
}

impl Function {
    /// Resolves a function name, ignoring case like Cypher does.
    pub(crate) fn resolve(name: &str) -> Result<Self, CypherError> {
        lookup(AGGREGATES, name)
            .map(Function::Aggregate)
            .or_else(|| lookup(SCALARS, name).map(Function::Scalar))
            .ok_or_else(|| CypherError::Unsupported(format!("function {name}()")))
    }
}

fn type_error(function: &str, value: &Value) -> CypherError {
    CypherError::TypeMismatch(format!(
        "{function}() does not accept {}",
        value.type_name()
    ))
}

fn arity(function: &str, args: &[Value], n: usize) -> Result<(), CypherError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(CypherError::InvalidQuery(format!(
            "{function}() expects {n} argument(s), got {}",
            args.len()
        )))
    }
}

/// `x` truncated towards zero; `None` when that does not fit an `i64` (or `x`
/// is not finite).
fn to_i64(x: f64) -> Option<i64> {
    const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
    let t = x.trunc();
    (-TWO_POW_63..TWO_POW_63).contains(&t).then_some(t as i64)
}

fn int_arg(function: &str, value: &Value) -> Result<Option<i64>, CypherError> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(i) => Ok(Some(*i)),
        Value::Float(f) if f.fract() == 0.0 => to_i64(*f)
            .map(Some)
            .ok_or_else(|| CypherError::RuntimeError("integer overflow".into())),
        other => Err(type_error(function, other)),
    }
}

fn float_fn(name: &str, args: &[Value], f: impl Fn(f64) -> f64) -> Result<Value, CypherError> {
    arity(name, args, 1)?;
    match &args[0] {
        Value::Null => Ok(Value::Null),
        v => v
            .as_f64()
            .map(|x| Value::Float(f(x)))
            .ok_or_else(|| type_error(name, v)),
    }
}

fn string_fn(name: &str, args: &[Value], f: impl Fn(&str) -> String) -> Result<Value, CypherError> {
    arity(name, args, 1)?;
    match &args[0] {
        Value::Null => Ok(Value::Null),
        Value::String(s) => Ok(Value::String(f(s))),
        v => Err(type_error(name, v)),
    }
}

/// The plain conversion behind an `…OrNull` or `…List` function.
fn conversion(f: Scalar) -> Scalar {
    use Scalar as S;
    match f {
        S::ToBooleanOrNull | S::ToBooleanList => S::ToBoolean,
        S::ToIntegerOrNull | S::ToIntegerList => S::ToInteger,
        S::ToFloatOrNull | S::ToFloatList => S::ToFloat,
        _ => S::ToString,
    }
}

/// Runs a conversion, giving `null` for a value of a type it cannot convert.
fn convert_or_null(f: Scalar, value: Value) -> Result<Value, CypherError> {
    match call(f, vec![value]) {
        Err(CypherError::TypeMismatch(_)) => Ok(Value::Null),
        result => result,
    }
}

/// `trim`, `ltrim`, `rtrim` and `btrim`: whitespace, or the single character
/// given as second argument, removed from one or both ends.
fn trim(f: Scalar, args: &[Value]) -> Result<Value, CypherError> {
    let name = f.name();
    if args.is_empty() || args.len() > 2 {
        return Err(CypherError::InvalidQuery(format!(
            "{name}() expects 1 or 2 arguments, got {}",
            args.len()
        )));
    }
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    let Value::String(s) = &args[0] else {
        return Err(type_error(name, &args[0]));
    };
    let trim_char = match args.get(1) {
        None => None,
        Some(Value::String(t)) => {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => {
                    return Err(CypherError::RuntimeError(format!(
                        "{name}() expects a single trim character, got '{t}'"
                    )));
                }
            }
        }
        Some(v) => return Err(type_error(name, v)),
    };
    let strip = |c: char| trim_char.map_or(c.is_whitespace(), |t| c == t);
    Ok(Value::String(
        match f {
            Scalar::LTrim => s.trim_start_matches(strip),
            Scalar::RTrim => s.trim_end_matches(strip),
            _ => s.trim_matches(strip),
        }
        .to_owned(),
    ))
}

/// Splits `s` at every occurrence of any of `delimiters`, preferring the
/// longest delimiter that matches at a position. Empty delimiters are
/// ignored.
fn split_any(s: &str, delimiters: &[&str]) -> Vec<Value> {
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < s.len() {
        let matched = delimiters
            .iter()
            .filter(|d| !d.is_empty() && s[i..].starts_with(**d))
            .map(|d| d.len())
            .max();
        match matched {
            Some(len) => {
                parts.push(Value::from(&s[start..i]));
                i += len;
                start = i;
            }
            None => i += s[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    parts.push(Value::from(&s[start..]));
    parts
}

/// How `round(x, precision, mode)` treats the digits it drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoundingMode {
    /// Away from zero.
    Up,
    /// Towards zero.
    Down,
    /// Towards positive infinity.
    Ceiling,
    /// Towards negative infinity.
    Floor,
    /// To the nearest; ties away from zero.
    HalfUp,
    /// To the nearest; ties towards zero.
    HalfDown,
    /// To the nearest; ties to the even neighbour.
    HalfEven,
}

/// `round(x[, precision[, mode]])`.
fn round(args: &[Value]) -> Result<Value, CypherError> {
    if !(1..=3).contains(&args.len()) {
        return Err(CypherError::InvalidQuery(format!(
            "round() expects 1 to 3 arguments, got {}",
            args.len()
        )));
    }
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    let x = args[0]
        .as_f64()
        .ok_or_else(|| type_error("round", &args[0]))?;
    let precision = match args.get(1) {
        Some(p) => int_arg("round", p)?.unwrap_or(0),
        None => 0,
    };
    let mode = match args.get(2) {
        None => None,
        Some(Value::String(m)) => Some(match m.as_str() {
            "UP" => RoundingMode::Up,
            "DOWN" => RoundingMode::Down,
            "CEILING" => RoundingMode::Ceiling,
            "FLOOR" => RoundingMode::Floor,
            "HALF_UP" => RoundingMode::HalfUp,
            "HALF_DOWN" => RoundingMode::HalfDown,
            "HALF_EVEN" => RoundingMode::HalfEven,
            other => {
                return Err(CypherError::RuntimeError(format!(
                    "unknown rounding mode '{other}'"
                )));
            }
        }),
        Some(v) => return Err(type_error("round", v)),
    };
    Ok(Value::Float(match mode {
        // Without a mode, whole numbers keep Cypher's classic rounding of
        // ties towards positive infinity (`round(-1.5)` is -1.0), while a
        // precision rounds ties away from zero (`round(-1.55, 1)` is -1.6).
        // `x - floor` is exact; `(x + 0.5).floor()` would round twice.
        None if precision == 0 => {
            let f = x.floor();
            if x - f >= 0.5 { f + 1.0 } else { f }
        }
        None => round_decimal(x, precision, RoundingMode::HalfUp),
        Some(mode) => round_decimal(x, precision, mode),
    }))
}

/// Rounds `x` to `precision` decimal places (tens, hundreds, … when
/// negative). It works on the shortest decimal form of `x`, like Java's
/// `BigDecimal.valueOf(double)`, so `2.675` rounds to `2.68` although the
/// nearest double is slightly below 2.675.
fn round_decimal(x: f64, precision: i64, mode: RoundingMode) -> f64 {
    use std::cmp::Ordering;
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let negative = x < 0.0;
    // `d.ddd…e<exp>`: digit `i` has the place value 10^(exp - i).
    let formatted = format!("{:e}", x.abs());
    let (mantissa, exp) = formatted.split_once('e').expect("exponent notation");
    let exp: i64 = exp.parse().expect("integer exponent");
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // How many digits have a place value of at least 10^-precision.
    let keep = exp.saturating_add(precision).saturating_add(1);
    if keep >= digits.len() as i64 {
        return x;
    }
    let (kept, dropped) = digits.split_at(usize::try_from(keep).unwrap_or(0));
    // The first dropped digit sits right below the last kept place; when
    // `keep` is negative, zeros precede all digits.
    let (first, rest_nonzero) = if keep < 0 {
        (0, true)
    } else {
        (dropped[0], dropped[1..].iter().any(|&d| d != 0))
    };
    // The dropped part compared with half a unit of the last kept place.
    let half = match first.cmp(&5) {
        Ordering::Equal if rest_nonzero => Ordering::Greater,
        other => other,
    };
    let inexact = first != 0 || rest_nonzero;
    // At most 17 significant digits, so this fits.
    let mut value = kept.iter().fold(0u64, |v, &d| v * 10 + u64::from(d));
    let up = match mode {
        RoundingMode::Up => inexact,
        RoundingMode::Down => false,
        RoundingMode::Ceiling => inexact && !negative,
        RoundingMode::Floor => inexact && negative,
        RoundingMode::HalfUp => half != Ordering::Less,
        RoundingMode::HalfDown => half == Ordering::Greater,
        RoundingMode::HalfEven => {
            half == Ordering::Greater || (half == Ordering::Equal && value % 2 == 1)
        }
    };
    if up {
        value += 1;
    }
    let magnitude: f64 = format!("{value}e{}", precision.saturating_neg())
        .parse()
        .expect("decimal literal");
    if negative && value != 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// Calls a scalar function that only needs its argument values. Functions on
/// graph elements are evaluated by the executor.
pub(crate) fn call(f: Scalar, args: Vec<Value>) -> Result<Value, CypherError> {
    use Scalar as S;
    let name = f.name();
    match f {
        S::Coalesce => Ok(args.into_iter().find(|v| !v.is_null()).unwrap_or_default()),
        S::Size => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::List(l) => Ok(Value::Integer(l.len() as i64)),
                Value::String(s) => Ok(Value::Integer(s.chars().count() as i64)),
                v => Err(type_error("size", v)),
            }
        }
        S::Head | S::Last => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::List(l) => Ok(if f == S::Head { l.first() } else { l.last() }
                    .cloned()
                    .unwrap_or_default()),
                v => Err(type_error(name, v)),
            }
        }
        S::Tail => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::List(l) => Ok(Value::List(l.iter().skip(1).cloned().collect())),
                v => Err(type_error("tail", v)),
            }
        }
        S::Reverse => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::List(l) => Ok(Value::List(l.iter().rev().cloned().collect())),
                Value::String(s) => Ok(Value::String(s.chars().rev().collect())),
                v => Err(type_error("reverse", v)),
            }
        }
        S::Range => {
            if !(2..=3).contains(&args.len()) {
                return Err(CypherError::InvalidQuery(
                    "range() expects 2 or 3 arguments".into(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Value::Null);
            }
            let int = |v: &Value| match v {
                Value::Integer(i) => Ok(*i),
                other => Err(CypherError::TypeMismatch(format!(
                    "range() expects integers, got {}",
                    other.type_name()
                ))),
            };
            let (start, end) = (int(&args[0])?, int(&args[1])?);
            let step = args.get(2).map(int).transpose()?.unwrap_or(1);
            if step == 0 {
                return Err(CypherError::RuntimeError(
                    "range() step must not be 0".into(),
                ));
            }
            let mut out = Vec::new();
            let mut i = start;
            while (step > 0 && i <= end) || (step < 0 && i >= end) {
                out.push(Value::Integer(i));
                match i.checked_add(step) {
                    Some(next) => i = next,
                    None => break,
                }
            }
            Ok(Value::List(out))
        }
        S::ToBoolean => {
            arity(name, &args, 1)?;
            Ok(match &args[0] {
                Value::Boolean(b) => Value::Boolean(*b),
                Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    _ => Value::Null,
                },
                Value::Integer(i) => Value::Boolean(*i != 0),
                Value::Null => Value::Null,
                v => return Err(type_error("toBoolean", v)),
            })
        }
        S::ToInteger => {
            arity(name, &args, 1)?;
            Ok(match &args[0] {
                Value::Integer(i) => Value::Integer(*i),
                Value::Float(x) => to_i64(*x).map_or(Value::Null, Value::Integer),
                Value::Boolean(b) => Value::Integer(i64::from(*b)),
                Value::String(s) => {
                    let s = s.trim();
                    s.parse::<i64>()
                        .ok()
                        .or_else(|| s.parse::<f64>().ok().and_then(to_i64))
                        .map_or(Value::Null, Value::Integer)
                }
                Value::Null => Value::Null,
                v => return Err(type_error("toInteger", v)),
            })
        }
        S::ToFloat => {
            arity(name, &args, 1)?;
            Ok(match &args[0] {
                Value::Integer(i) => Value::Float(*i as f64),
                Value::Float(x) => Value::Float(*x),
                Value::String(s) => s.trim().parse::<f64>().map_or(Value::Null, Value::Float),
                Value::Null => Value::Null,
                v => return Err(type_error("toFloat", v)),
            })
        }
        S::ToString => {
            arity(name, &args, 1)?;
            Ok(match &args[0] {
                Value::String(s) => Value::String(s.clone()),
                Value::Integer(i) => Value::String(i.to_string()),
                Value::Float(x) => Value::String(format_float(*x)),
                Value::Boolean(b) => Value::String(b.to_string()),
                Value::Null => Value::Null,
                v => return Err(type_error("toString", v)),
            })
        }
        S::ToUpper => string_fn(name, &args, str::to_uppercase),
        S::ToLower => string_fn(name, &args, str::to_lowercase),
        S::Trim | S::LTrim | S::RTrim | S::BTrim => trim(f, &args),
        S::Substring => {
            if !(2..=3).contains(&args.len()) {
                return Err(CypherError::InvalidQuery(
                    "substring() expects 2 or 3 arguments".into(),
                ));
            }
            if args.iter().any(Value::is_null) {
                return Ok(Value::Null);
            }
            let s = match &args[0] {
                Value::Null => return Ok(Value::Null),
                Value::String(s) => s,
                v => return Err(type_error("substring", v)),
            };
            let start = int_arg("substring", &args[1])?.unwrap_or(0);
            let len = args
                .get(2)
                .map(|v| int_arg("substring", v))
                .transpose()?
                .flatten();
            if start < 0 || len.is_some_and(|l| l < 0) {
                return Err(CypherError::RuntimeError(
                    "substring() expects non-negative start and length".into(),
                ));
            }
            let chars = s.chars().skip(start as usize);
            Ok(Value::String(match len {
                Some(l) => chars.take(l as usize).collect(),
                None => chars.collect(),
            }))
        }
        S::Left | S::Right => {
            arity(name, &args, 2)?;
            if args.iter().any(Value::is_null) {
                return Ok(Value::Null);
            }
            let s = match &args[0] {
                Value::Null => return Ok(Value::Null),
                Value::String(s) => s,
                v => return Err(type_error(name, v)),
            };
            let n = match int_arg(name, &args[1])? {
                Some(n) if n >= 0 => n as usize,
                Some(_) => {
                    return Err(CypherError::RuntimeError(format!(
                        "{name}() expects a non-negative length"
                    )));
                }
                None => {
                    return Err(CypherError::RuntimeError(format!(
                        "{name}() expects a length"
                    )));
                }
            };
            let count = s.chars().count();
            Ok(Value::String(if f == S::Left {
                s.chars().take(n).collect()
            } else {
                s.chars().skip(count.saturating_sub(n)).collect()
            }))
        }
        S::Replace => {
            arity(name, &args, 3)?;
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to)))
                }
                (a, b, c) if a.is_null() || b.is_null() || c.is_null() => Ok(Value::Null),
                (v, _, _) => Err(type_error("replace", v)),
            }
        }
        S::Split => {
            arity(name, &args, 2)?;
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(sep)) => Ok(Value::List(
                    s.split(sep.as_str())
                        .map(|p| Value::String(p.to_owned()))
                        .collect(),
                )),
                (a, b) if a.is_null() || b.is_null() => Ok(Value::Null),
                (Value::String(s), Value::List(seps)) => {
                    let mut delimiters = Vec::with_capacity(seps.len());
                    for sep in seps {
                        match sep {
                            Value::String(d) => delimiters.push(d.as_str()),
                            Value::Null => return Ok(Value::Null),
                            v => return Err(type_error("split", v)),
                        }
                    }
                    Ok(Value::List(split_any(s, &delimiters)))
                }
                (v, _) => Err(type_error("split", v)),
            }
        }
        S::Abs => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Integer(i) => i
                    .checked_abs()
                    .map(Value::Integer)
                    .ok_or_else(|| CypherError::RuntimeError("integer overflow".into())),
                Value::Float(x) => Ok(Value::Float(x.abs())),
                v => Err(type_error("abs", v)),
            }
        }
        S::Sign => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Integer(i) => Ok(Value::Integer(i.signum())),
                Value::Float(x) if *x == 0.0 => Ok(Value::Integer(0)),
                Value::Float(x) => Ok(Value::Integer(if *x > 0.0 { 1 } else { -1 })),
                v => Err(type_error("sign", v)),
            }
        }
        S::Ceil => float_fn(name, &args, f64::ceil),
        S::Floor => float_fn(name, &args, f64::floor),
        S::Round => round(&args),
        S::Sqrt => float_fn(name, &args, f64::sqrt),
        S::Exp => float_fn(name, &args, f64::exp),
        S::Log => float_fn(name, &args, f64::ln),
        S::Log10 => float_fn(name, &args, f64::log10),
        S::Sin => float_fn(name, &args, f64::sin),
        S::Cos => float_fn(name, &args, f64::cos),
        S::Tan => float_fn(name, &args, f64::tan),
        S::Cot => float_fn(name, &args, |x| 1.0 / x.tan()),
        S::Asin => float_fn(name, &args, f64::asin),
        S::Acos => float_fn(name, &args, f64::acos),
        S::Atan => float_fn(name, &args, f64::atan),
        S::Degrees => float_fn(name, &args, f64::to_degrees),
        S::Radians => float_fn(name, &args, f64::to_radians),
        S::Haversin => float_fn(name, &args, |x| (1.0 - x.cos()) / 2.0),
        S::Atan2 => {
            arity(name, &args, 2)?;
            match (args[0].as_f64(), args[1].as_f64()) {
                (Some(y), Some(x)) => Ok(Value::Float(y.atan2(x))),
                _ if args[0].is_null() || args[1].is_null() => Ok(Value::Null),
                _ => Err(type_error("atan2", &args[0])),
            }
        }
        S::E => Ok(Value::Float(std::f64::consts::E)),
        S::Pi => Ok(Value::Float(std::f64::consts::PI)),
        S::Rand => rand(),
        S::ToBooleanOrNull | S::ToIntegerOrNull | S::ToFloatOrNull | S::ToStringOrNull => {
            arity(name, &args, 1)?;
            let value = args.into_iter().next().unwrap_or_default();
            convert_or_null(conversion(f), value)
        }
        S::ToBooleanList | S::ToIntegerList | S::ToFloatList | S::ToStringList => {
            arity(name, &args, 1)?;
            match args.into_iter().next().unwrap_or_default() {
                Value::Null => Ok(Value::Null),
                Value::List(items) => Ok(Value::List(
                    items
                        .into_iter()
                        .map(|v| convert_or_null(conversion(f), v))
                        .collect::<Result<_, _>>()?,
                )),
                v => Err(type_error(name, &v)),
            }
        }
        S::IsEmpty => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::List(l) => Ok(Value::Boolean(l.is_empty())),
                Value::Map(m) => Ok(Value::Boolean(m.is_empty())),
                Value::String(s) => Ok(Value::Boolean(s.is_empty())),
                v => Err(type_error(name, v)),
            }
        }
        S::IsNaN => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Integer(_) => Ok(Value::Boolean(false)),
                Value::Float(x) => Ok(Value::Boolean(x.is_nan())),
                v => Err(type_error(name, v)),
            }
        }
        S::CharLength => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::Integer(s.chars().count() as i64)),
                v => Err(type_error(name, v)),
            }
        }
        S::ElementId => {
            arity(name, &args, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Node(n) => Ok(Value::String(format!("node:{}", n.index()))),
                Value::Relationship(e) => Ok(Value::String(format!("relationship:{}", e.index()))),
                v => Err(type_error(name, v)),
            }
        }
        S::ValueType => {
            arity(name, &args, 1)?;
            Ok(Value::String(super::value_type::value_type(&args[0])))
        }
        S::RandomUuid => {
            arity(name, &args, 0)?;
            Ok(Value::String(random_uuid()))
        }
        S::NullIf => {
            arity(name, &args, 2)?;
            let mut args = args.into_iter();
            let (a, b) = (
                args.next().unwrap_or_default(),
                args.next().unwrap_or_default(),
            );
            Ok(if a.equivalent(&b) { Value::Null } else { a })
        }
        S::Exists
        | S::Id
        | S::Labels
        | S::Type
        | S::Properties
        | S::Keys
        | S::StartNode
        | S::EndNode
        | S::Nodes
        | S::Relationships
        | S::Length
        | S::Timestamp => unreachable!("evaluated by the executor"),
    }
}

/// Renders a float the way `toString` does: always with a fractional part.
pub(crate) fn format_float(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else {
        format!("{x:?}")
    }
}

/// 64 random bits. Not cryptographic: every `RandomState` gets fresh SipHash
/// keys, so hashing nothing yields new random bits.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher, RandomState};
    RandomState::new().build_hasher().finish()
}

/// A uniform float in `[0, 1)`.
fn rand() -> Result<Value, CypherError> {
    Ok(Value::Float(
        (random_u64() >> 11) as f64 / (1u64 << 53) as f64,
    ))
}

/// A random (version 4) UUID, lowercase and hyphenated.
fn random_uuid() -> String {
    let mut bits = (u128::from(random_u64()) << 64) | u128::from(random_u64());
    bits = (bits & !(0xF << 76)) | (0x4 << 76); // version 4
    bits = (bits & !(0x3 << 62)) | (0x2 << 62); // RFC 4122 variant
    let hex = format!("{bits:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The running state of one aggregate over one group.
pub(crate) struct Accumulator {
    function: Aggregate,
    distinct: bool,
    values: Vec<Value>,
    /// The values seen so far, for `DISTINCT`.
    seen: BTreeSet<Ordered>,
    count: i64,
    /// The second argument of `percentileCont` / `percentileDisc`.
    percentile: Option<Value>,
}

impl Accumulator {
    pub(crate) fn new(function: Aggregate, distinct: bool) -> Self {
        Self {
            function,
            distinct,
            values: Vec::new(),
            seen: BTreeSet::new(),
            count: 0,
            percentile: None,
        }
    }

    /// Adds one row. `None` is `count(*)`, which counts every row.
    pub(crate) fn add(&mut self, args: Option<Vec<Value>>) {
        let Some(mut args) = args else {
            self.count += 1;
            return;
        };
        if self.function.is_percentile() && args.len() > 1 {
            self.percentile = Some(args.swap_remove(1));
        }
        let Some(value) = args.into_iter().next() else {
            return;
        };
        if value.is_null() {
            return;
        }
        if self.distinct && !self.seen.insert(Ordered(vec![value.clone()])) {
            return;
        }
        self.count += 1;
        if self.function != Aggregate::Count {
            self.values.push(value);
        }
    }

    pub(crate) fn finish(self) -> Result<Value, CypherError> {
        use Aggregate as A;
        let numbers = |name: &str| {
            self.values
                .iter()
                .map(|v| v.as_f64().ok_or_else(|| type_error(name, v)))
                .collect::<Result<Vec<f64>, _>>()
        };
        Ok(match self.function {
            A::Count => Value::Integer(self.count),
            A::Collect => Value::List(self.values),
            A::Min => self
                .values
                .into_iter()
                .min_by(Value::order)
                .unwrap_or_default(),
            A::Max => self
                .values
                .into_iter()
                .max_by(Value::order)
                .unwrap_or_default(),
            A::Sum => {
                if self.values.iter().all(|v| matches!(v, Value::Integer(_))) {
                    let mut sum: i64 = 0;
                    for v in &self.values {
                        if let Value::Integer(i) = v {
                            sum = sum.checked_add(*i).ok_or_else(|| {
                                CypherError::RuntimeError("integer overflow".into())
                            })?;
                        }
                    }
                    Value::Integer(sum)
                } else {
                    Value::Float(numbers("sum")?.iter().sum())
                }
            }
            A::Avg => {
                let xs = numbers("avg")?;
                if xs.is_empty() {
                    Value::Null
                } else {
                    Value::Float(xs.iter().sum::<f64>() / xs.len() as f64)
                }
            }
            A::StDev | A::StDevP | A::Var | A::VarP => {
                let xs = numbers("stDev")?;
                let n = xs.len() as f64;
                // Sample statistics divide by n - 1, population ones by n.
                let denominator = if matches!(self.function, A::StDev | A::Var) {
                    n - 1.0
                } else {
                    n
                };
                let var = if denominator <= 0.0 {
                    0.0
                } else {
                    let mean = xs.iter().sum::<f64>() / n;
                    xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / denominator
                };
                Value::Float(if matches!(self.function, A::Var | A::VarP) {
                    var
                } else {
                    var.sqrt()
                })
            }
            A::PercentileCont | A::PercentileDisc => {
                let p = match &self.percentile {
                    Some(v) => v.as_f64().ok_or_else(|| type_error("percentile", v))?,
                    None => return Ok(Value::Null),
                };
                if !(0.0..=1.0).contains(&p) {
                    return Err(CypherError::RuntimeError(
                        "percentile must be between 0.0 and 1.0".into(),
                    ));
                }
                if self.values.is_empty() {
                    return Ok(Value::Null);
                }
                if self.function == A::PercentileDisc {
                    // Returns one of the inputs, so no float round trip.
                    if let Some(v) = self.values.iter().find(|v| v.as_f64().is_none()) {
                        return Err(type_error("percentile", v));
                    }
                    let mut values = self.values;
                    values.sort_by(Value::order);
                    let idx = ((p * values.len() as f64).ceil() as usize).saturating_sub(1);
                    values.swap_remove(idx.min(values.len() - 1))
                } else {
                    let mut xs = numbers("percentile")?;
                    xs.sort_by(|a, b| a.total_cmp(b));
                    let rank = p * (xs.len() - 1) as f64;
                    let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
                    Value::Float(xs[lo] + (xs[hi] - xs[lo]) * (rank - lo as f64))
                }
            }
        })
    }
}
