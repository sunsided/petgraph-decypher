//! nom-based parser for a useful subset of the OpenCypher query language.

use std::collections::HashMap;

use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_until, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt, recognize, value},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, terminated, tuple},
};

use crate::ast::*;
use crate::error::CypherError;

/// Match an identifier: `[a-zA-Z_][a-zA-Z0-9_]*`
fn identifier(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        many0(take_while1(|c: char| c.is_alphanumeric() || c == '_')),
    ))(input)
}

/// Match a backtick-quoted identifier: `` `any-thing!` ``
fn backtick_identifier(input: &str) -> IResult<&str, &str> {
    delimited(char('`'), take_while1(|c: char| c != '`'), char('`'))(input)
}

/// Match either a regular identifier or a backtick-quoted identifier.
fn any_identifier(input: &str) -> IResult<&str, &str> {
    alt((backtick_identifier, identifier))(input)
}

/// Match a case-insensitive keyword, asserting it is not followed by an
/// identifier character (so `MATCH` does not accidentally match `MATCHES`).
fn keyword<'a>(kw: &'static str) -> impl Fn(&'a str) -> IResult<&'a str, ()> {
    move |input| {
        let (rest, _) = tag_no_case(kw)(input)?;
        if rest
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
        Ok((rest, ()))
    }
}

/// Parse a double- or single-quoted string literal into a `CypherValue::String`.
fn string_literal(input: &str) -> IResult<&str, CypherValue> {
    alt((
        map(
            delimited(char('"'), take_until("\""), char('"')),
            |s: &str| CypherValue::String(s.to_string()),
        ),
        map(
            delimited(char('\''), take_until("'"), char('\'')),
            |s: &str| CypherValue::String(s.to_string()),
        ),
    ))(input)
}

/// Parse a signed floating-point literal (e.g. `-3.14`) into a `CypherValue::Float`.
fn float_literal(input: &str) -> IResult<&str, CypherValue> {
    map(
        recognize(tuple((opt(char('-')), digit1, char('.'), digit1))),
        |s: &str| {
            CypherValue::Float(
                s.parse::<f64>()
                    .expect("recognize guarantees valid float digits"),
            )
        },
    )(input)
}

/// Parse a signed integer literal (e.g. `42`, `-7`) into a `CypherValue::Integer`.
fn integer_literal(input: &str) -> IResult<&str, CypherValue> {
    map(recognize(pair(opt(char('-')), digit1)), |s: &str| {
        CypherValue::Integer(
            s.parse::<i64>()
                .expect("recognize guarantees valid integer digits"),
        )
    })(input)
}

/// Parse the `true` or `false` keyword into a `CypherValue::Boolean`.
fn boolean_literal(input: &str) -> IResult<&str, CypherValue> {
    alt((
        value(CypherValue::Boolean(true), keyword("true")),
        value(CypherValue::Boolean(false), keyword("false")),
    ))(input)
}

/// Parse the `null` keyword into a `CypherValue::Null`.
fn null_literal(input: &str) -> IResult<&str, CypherValue> {
    value(CypherValue::Null, keyword("null"))(input)
}

/// Parse any Cypher literal value.
fn cypher_value(input: &str) -> IResult<&str, CypherValue> {
    alt((
        boolean_literal,
        null_literal,
        string_literal,
        float_literal, // must precede integer_literal
        integer_literal,
    ))(input)
}

/// Parse a parameter reference: `$param_name`.
fn parameter_ref(input: &str) -> IResult<&str, Expression> {
    map(
        preceded(char('$'), map(any_identifier, str::to_string)),
        |name| Expression::Parameter(name),
    )(input)
}

/// Parse a list literal: `[expr, expr, ...]`.
fn list_literal(input: &str) -> IResult<&str, Expression> {
    map(
        delimited(
            pair(char('['), multispace0),
            separated_list0(tuple((multispace0, char(','), multispace0)), expression),
            pair(multispace0, char(']')),
        ),
        |exprs| Expression::List(exprs),
    )(input)
}

/// Parse a map literal: `{key: expr, key: expr}`.
/// Note: This is distinct from the properties parser which only accepts cypher_value.
fn map_literal(input: &str) -> IResult<&str, Expression> {
    let kv_pair = map(
        tuple((
            preceded(multispace0, map(any_identifier, str::to_string)),
            preceded(multispace0, char(':')),
            preceded(multispace0, expression),
        )),
        |(key, _, val)| (key, val),
    );

    map(
        delimited(
            pair(char('{'), multispace0),
            separated_list0(tuple((multispace0, char(','), multispace0)), kv_pair),
            pair(multispace0, char('}')),
        ),
        |pairs| Expression::Map(pairs),
    )(input)
}

/// Parse a function call: `name(args)`.
fn function_call(input: &str) -> IResult<&str, Expression> {
    map(
        pair(
            map(any_identifier, str::to_string),
            delimited(
                pair(char('('), multispace0),
                separated_list0(tuple((multispace0, char(','), multispace0)), expression),
                pair(multispace0, char(')')),
            ),
        ),
        |(name, args)| Expression::FunctionCall { name, args },
    )(input)
}

/// Parse a single `key: value` pair inside a property map.
fn property_pair(input: &str) -> IResult<&str, (String, CypherValue)> {
    map(
        tuple((
            preceded(multispace0, map(identifier, str::to_string)),
            preceded(multispace0, char(':')),
            preceded(multispace0, cypher_value),
        )),
        |(key, _, val)| (key, val),
    )(input)
}

/// Parse a `{key: value, ...}` property map into a `HashMap`.
fn properties(input: &str) -> IResult<&str, HashMap<String, CypherValue>> {
    map(
        delimited(
            pair(char('{'), multispace0),
            separated_list0(tuple((multispace0, char(','), multispace0)), property_pair),
            pair(multispace0, char('}')),
        ),
        |pairs| pairs.into_iter().collect(),
    )(input)
}

/// Parse a node label prefixed with `:` (e.g. `:Person`).
fn label(input: &str) -> IResult<&str, String> {
    map(preceded(char(':'), identifier), str::to_string)(input)
}

/// Parse a node pattern `(var:Label1:Label2 {props})`.
fn node_pattern(input: &str) -> IResult<&str, NodePattern> {
    delimited(
        pair(char('('), multispace0),
        map(
            tuple((
                opt(terminated(
                    map(identifier, str::to_string),
                    // The variable must be followed by ':', '{', ')', or whitespace —
                    // not immediately a ':' that belongs to a label on an anonymous node.
                    // We use `opt` on `identifier` so the peek is implicit.
                    multispace0,
                )),
                opt(many1(preceded(multispace0, label))),
                opt(preceded(multispace0, properties)),
            )),
            |(var, lbls, props)| NodePattern {
                variable: var,
                labels: lbls.unwrap_or_default(),
                properties: props.unwrap_or_default(),
            },
        ),
        pair(multispace0, char(')')),
    )(input)
}

/// Parse relationship types prefixed with `:` (e.g. `:KNOWS` or `:KNOWS|FRIENDS`).
fn rel_type_specs(input: &str) -> IResult<&str, Vec<String>> {
    preceded(
        char(':'),
        separated_list1(char('|'), map(identifier, str::to_string)),
    )(input)
}

/// The inner components of a relationship bracket: `(variable, types, variable_length, properties)`.
type RelInner = (Option<String>, Vec<String>, Option<VariableLength>, HashMap<String, CypherValue>);

/// Parse a variable-length quantifier: `*`, `*n`, `*min..max`, `*min..`, `*..max`.
fn variable_length(input: &str) -> IResult<&str, VariableLength> {
    let (rest, _) = char('*')(input)?;
    let (rest, _) = multispace0(rest)?;

    // Try to parse optional min digits
    let (rest, has_digits) = opt(digit1)(rest)?;
    let (rest, _) = multispace0(rest)?;

    if has_digits.is_none() {
        // Check for `..` (max only: `*..max`)
        let (rest, has_dotdot) = opt(tag(".."))(rest)?;
        if has_dotdot.is_some() {
            let (rest, _) = multispace0(rest)?;
            let (rest, max_str) = digit1(rest)?;
            let max: u64 = max_str.parse().expect("digit1 guarantees valid number");
            return Ok((rest, VariableLength { min: Some(0), max: Some(max) }));
        }
        // Bare `*` — unbounded, min=0
        return Ok((rest, VariableLength { min: Some(0), max: None }));
    }

    let min_str = has_digits.unwrap();
    let min: u64 = min_str.parse().expect("digit1 guarantees valid number");

    // Check for `..`
    let (rest, has_dotdot) = opt(tag(".."))(rest)?;

    if has_dotdot.is_none() {
        // Exact length: `*n`
        return Ok((rest, VariableLength { min: Some(min), max: Some(min) }));
    }

    let (rest, _) = multispace0(rest)?;
    let (rest, max_str) = opt(digit1)(rest)?;

    let max = match max_str {
        Some(s) => Some(s.parse().expect("digit1 guarantees valid number")),
        None => None,
    };

    Ok((rest, VariableLength { min: Some(min), max }))
}

/// Parse the inner part of `[var:TYPE|TYPE2*1..5 {props}]`.
fn rel_inner(input: &str) -> IResult<&str, RelInner> {
    map(
        tuple((
            opt(map(identifier, str::to_string)),
            opt(preceded(multispace0, rel_type_specs)),
            opt(preceded(multispace0, variable_length)),
            opt(preceded(multispace0, properties)),
        )),
        |(var, rtypes, vlen, props)| (var, rtypes.unwrap_or_default(), vlen, props.unwrap_or_default()),
    )(input)
}

/// `-[...]->`  or  `-[...]-`
fn rel_right_or_undir(input: &str) -> IResult<&str, RelPattern> {
    let (rest, _) = tag("-[")(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, (var, rtypes, vlen, props)) = rel_inner(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, direction) = alt((
        value(RelDirection::Right, tag("]->")),
        value(RelDirection::Both, tag("]-")),
    ))(rest)?;
    Ok((
        rest,
        RelPattern {
            variable: var,
            rel_types: rtypes,
            properties: props,
            direction,
            variable_length: vlen,
        },
    ))
}

/// `<-[...]-`
fn rel_left_bracketed(input: &str) -> IResult<&str, RelPattern> {
    let (rest, _) = tag("<-[")(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, (var, rtypes, vlen, props)) = rel_inner(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, _) = tag("]-")(rest)?;
    Ok((
        rest,
        RelPattern {
            variable: var,
            rel_types: rtypes,
            properties: props,
            direction: RelDirection::Left,
            variable_length: vlen,
        },
    ))
}

/// Any relationship pattern form.
fn rel_pattern(input: &str) -> IResult<&str, RelPattern> {
    alt((
        rel_left_bracketed,                                         // <-[...]-
        rel_right_or_undir,                                         // -[...]-> or -[...]-
        value(RelPattern::simple(RelDirection::Right), tag("-->")), // -->
        value(RelPattern::simple(RelDirection::Left), tag("<--")),  // <--
        value(RelPattern::simple(RelDirection::Both), tag("--")),   // --
    ))(input)
}

/// Parse a path pattern: a start node followed by zero or more (relationship, node) hops.
fn path_pattern(input: &str) -> IResult<&str, PathPattern> {
    map(
        pair(
            node_pattern,
            many0(pair(
                preceded(multispace0, rel_pattern),
                preceded(multispace0, node_pattern),
            )),
        ),
        |(start, rels)| PathPattern { start, rels },
    )(input)
}

/// Parse a CASE expression: `CASE WHEN cond THEN result [WHEN ...] [ELSE default] END`.
fn case_expression(input: &str) -> IResult<&str, Expression> {
    let arm = map(
        tuple((
            preceded(
                tuple((multispace1, keyword("WHEN"), multispace1)),
                expression,
            ),
            preceded(
                tuple((multispace1, keyword("THEN"), multispace1)),
                expression,
            ),
        )),
        |(cond, result)| (Box::new(cond), Box::new(result)),
    );

    let default = preceded(
        tuple((multispace1, keyword("ELSE"), multispace1)),
        expression,
    );

    map(
        tuple((
            keyword("CASE"),
            many1(arm),
            opt(map(default, Box::new)),
            preceded(multispace1, keyword("END")),
        )),
        |(_, arms, default, _)| Expression::Case { arms, default },
    )(input)
}

/// Parse a primary expression: the most basic building blocks.
fn primary_expression(input: &str) -> IResult<&str, Expression> {
    alt((
        // Parenthesized expression
        delimited(
            pair(char('('), multispace0),
            expression,
            pair(multispace0, char(')')),
        ),
        // Parameter reference (must come before list literal to avoid $ confusion)
        parameter_ref,
        // List literal
        list_literal,
        // CASE expression (must come before keyword-based alternatives)
        case_expression,
        // Function call (must come before identifier to capture `name(...)`)
        function_call,
        // Map literal (must come before property-based patterns)
        map_literal,
        // Literal value as expression
        map(cypher_value, Expression::Literal),
        // Wildcard
        value(Expression::All, char('*')),
        // Identifier (variable, possibly with property access)
        map(
            pair(
                map(any_identifier, str::to_string),
                many0(preceded(
                    char('.'),
                    preceded(multispace0, map(any_identifier, str::to_string)),
                )),
            ),
            |(var, props)| {
                if props.is_empty() {
                    Expression::Variable(var)
                } else if props.len() == 1 {
                    Expression::Property(var, props.into_iter().next().unwrap())
                } else {
                    Expression::NestedProperty {
                        variable: var,
                        properties: props,
                    }
                }
            },
        ),
    ))(input)
}

/// Parse a multiplicative expression: `*`, `/`, `%`.
fn multiplicative_expression(input: &str) -> IResult<&str, Expression> {
    let (rest, left) = primary_expression(input)?;
    let (rest, ops) = many0(tuple((
        multispace0,
        alt((
            value("mul", char('*')),
            value("div", char('/')),
            value("mod", char('%')),
        )),
        multispace0,
        primary_expression,
    )))(rest)?;

    let result = ops
        .into_iter()
        .fold(left, |acc, (_, op, _, right)| match op {
            "mul" => Expression::Mul(Box::new(acc), Box::new(right)),
            "div" => Expression::Div(Box::new(acc), Box::new(right)),
            "mod" => Expression::Mod(Box::new(acc), Box::new(right)),
            _ => unreachable!(),
        });
    Ok((rest, result))
}

/// Parse an additive expression: `+`, `-`.
fn additive_expression(input: &str) -> IResult<&str, Expression> {
    let (rest, left) = multiplicative_expression(input)?;
    let (rest, ops) = many0(tuple((
        multispace0,
        alt((value("add", char('+')), value("sub", char('-')))),
        multispace0,
        multiplicative_expression,
    )))(rest)?;

    let result = ops
        .into_iter()
        .fold(left, |acc, (_, op, _, right)| match op {
            "add" => Expression::Add(Box::new(acc), Box::new(right)),
            "sub" => Expression::Sub(Box::new(acc), Box::new(right)),
            _ => unreachable!(),
        });
    Ok((rest, result))
}

/// Parse a comparison expression: `>`, `<`, `>=`, `<=`, `<>`, `!=`.
fn comparison_expression(input: &str) -> IResult<&str, Expression> {
    let (rest, left) = additive_expression(input)?;

    fn comparison_op(input: &str) -> IResult<&str, &str> {
        alt((
            tag("<>"),
            tag("!="),
            tag("<="),
            tag(">="),
            tag("<"),
            tag(">"),
        ))(input)
    }

    let attempt = pair(
        preceded(multispace0::<&str, _>, comparison_op),
        pair(multispace0::<&str, _>, additive_expression),
    )(rest);

    match attempt {
        Ok((new_rest, (op, (_, right)))) => {
            let result = match op {
                "<" => Expression::Lt(Box::new(left), Box::new(right)),
                ">" => Expression::Gt(Box::new(left), Box::new(right)),
                "<=" => Expression::Lte(Box::new(left), Box::new(right)),
                ">=" => Expression::Gte(Box::new(left), Box::new(right)),
                "<>" | "!=" => Expression::Neq(Box::new(left), Box::new(right)),
                _ => unreachable!(),
            };
            Ok((new_rest, result))
        }
        Err(_) => Ok((rest, left)),
    }
}

/// Parse an expression with full precedence.
/// Handles: literals, variables, property access, arithmetic, comparison,
/// function calls, list/map literals, parameters, CASE expressions.
fn expression(input: &str) -> IResult<&str, Expression> {
    comparison_expression(input)
}

/// Parse a single RETURN item with an optional `AS alias`.
fn return_item(input: &str) -> IResult<&str, ReturnItem> {
    map(
        pair(
            expression,
            opt(preceded(
                tuple((multispace1, keyword("AS"), multispace1)),
                map(identifier, str::to_string),
            )),
        ),
        |(expr, alias)| ReturnItem {
            expression: expr,
            alias,
        },
    )(input)
}

/// Parse a comparison operator followed by a literal value or expression.
fn where_comparison(input: &str) -> IResult<&str, WhereExpr> {
    // Use additive_expression (no comparisons) for the left side,
    // since the comparison operator is handled by WHERE itself.
    let (rest, expr) = preceded(multispace0, additive_expression)(input)?;
    let (rest, _) = multispace0(rest)?;

    // Try longer operators first to avoid `<` consuming before `<=`
    let (rest, op) = alt((
        value("neq", tag("<>")),
        value("neq", tag("!=")),
        value("lte", tag("<=")),
        value("gte", tag(">=")),
        value("lt", char('<')),
        value("gt", char('>')),
        value("eq", char('=')),
    ))(rest)?;

    let (rest, _) = multispace0(rest)?;
    // Try literal value first, then parameter reference
    let (rest, val) = alt((
        cypher_value,
        map(
            preceded(char('$'), map(any_identifier, str::to_string)),
            |_| CypherValue::Null, // Parameter, value resolved at runtime
        ),
    ))(rest)?;

    match op {
        "eq" => Ok((rest, WhereExpr::Eq(expr, val))),
        "neq" => Ok((rest, WhereExpr::Neq(expr, val))),
        "lt" => Ok((rest, WhereExpr::Lt(expr, val))),
        "gt" => Ok((rest, WhereExpr::Gt(expr, val))),
        "lte" => Ok((rest, WhereExpr::Lte(expr, val))),
        "gte" => Ok((rest, WhereExpr::Gte(expr, val))),
        _ => unreachable!(),
    }
}

/// Parse `IS NULL` or `IS NOT NULL`.
fn where_is_null(input: &str) -> IResult<&str, WhereExpr> {
    map(
        tuple((
            preceded(multispace0, expression),
            preceded(multispace1, keyword("IS")),
            opt(preceded(multispace1, keyword("NOT"))),
            preceded(multispace1, keyword("NULL")),
        )),
        |(expr, _, is_not, _)| {
            if is_not.is_some() {
                WhereExpr::IsNotNull(expr)
            } else {
                WhereExpr::IsNull(expr)
            }
        },
    )(input)
}

/// Parse `STARTS WITH`, `ENDS WITH`, or `CONTAINS`.
fn where_string_op(input: &str) -> IResult<&str, WhereExpr> {
    alt((
        map(
            tuple((
                preceded(multispace0, expression),
                preceded(multispace1, keyword("STARTS")),
                preceded(multispace1, keyword("WITH")),
                preceded(multispace1, expression),
            )),
            |(left, _, _, right)| WhereExpr::StartsWith(left, right),
        ),
        map(
            tuple((
                preceded(multispace0, expression),
                preceded(multispace1, keyword("ENDS")),
                preceded(multispace1, keyword("WITH")),
                preceded(multispace1, expression),
            )),
            |(left, _, _, right)| WhereExpr::EndsWith(left, right),
        ),
        map(
            tuple((
                preceded(multispace0, expression),
                preceded(multispace1, keyword("CONTAINS")),
                preceded(multispace1, expression),
            )),
            |(left, _, right)| WhereExpr::Contains(left, right),
        ),
    ))(input)
}

/// Parse `expr IN list_expr`.
fn where_in(input: &str) -> IResult<&str, WhereExpr> {
    map(
        tuple((
            preceded(multispace0, expression),
            preceded(multispace1, keyword("IN")),
            preceded(multispace1, expression),
        )),
        |(left, _, right)| WhereExpr::In(left, right),
    )(input)
}

/// Parse a NOT-prefixed WHERE expression.
fn where_not(input: &str) -> IResult<&str, WhereExpr> {
    map(
        preceded(
            tuple((multispace0, keyword("NOT"), multispace1)),
            where_atom,
        ),
        |inner| WhereExpr::Not(Box::new(inner)),
    )(input)
}

/// Parse a parenthesized WHERE expression.
fn where_paren(input: &str) -> IResult<&str, WhereExpr> {
    delimited(
        pair(char('('), multispace0),
        where_expr,
        pair(multispace0, char(')')),
    )(input)
}

/// Parse a single WHERE atom (the highest-precedence unit).
fn where_atom(input: &str) -> IResult<&str, WhereExpr> {
    alt((
        where_paren,
        where_not,
        where_is_null,
        where_string_op,
        where_in,
        where_comparison,
    ))(input)
}

/// Parse a WHERE expression with proper precedence:
/// - NOT (prefix, highest)
/// - Comparisons, IS NULL, string ops, IN
/// - AND
/// - OR (lowest)
fn where_expr(input: &str) -> IResult<&str, WhereExpr> {
    // Parse first atom
    let (rest, first) = where_atom(input)?;

    // Collect AND/OR chained expressions
    let (rest, rest_exprs) = many0(tuple((
        multispace1,
        alt((value("and", keyword("AND")), value("or", keyword("OR")))),
        multispace1,
        where_atom,
    )))(rest)?;

    // Fold: AND binds tighter than OR
    // Strategy: split by OR, then fold each group by AND
    let mut groups: Vec<Vec<WhereExpr>> = vec![vec![first]];

    for (_, op, _, right) in rest_exprs {
        if op == "or" {
            groups.push(vec![right]);
        } else {
            groups.last_mut().unwrap().push(right);
        }
    }

    // Fold each group by AND
    let and_folded: Vec<WhereExpr> = groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .reduce(|acc, e| WhereExpr::And(Box::new(acc), Box::new(e)))
                .unwrap()
        })
        .collect();

    // Fold by OR
    let result = and_folded
        .into_iter()
        .reduce(|acc, e| WhereExpr::Or(Box::new(acc), Box::new(e)))
        .unwrap();

    Ok((rest, result))
}

/// Parse a `WHERE` keyword followed by a `where_expr`.
fn where_clause(input: &str) -> IResult<&str, WhereExpr> {
    preceded(pair(keyword("WHERE"), multispace1), where_expr)(input)
}

/// Parse a `MATCH` clause with comma-separated path patterns and an optional `WHERE` condition.
fn match_clause(input: &str) -> IResult<&str, Clause> {
    let (rest, _) = keyword("MATCH")(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, patterns) =
        separated_list1(tuple((multispace0, char(','), multispace0)), path_pattern)(rest)?;
    let (rest, wc) = opt(preceded(multispace1, where_clause))(rest)?;
    Ok((
        rest,
        Clause::Match {
            patterns,
            where_clause: wc,
        },
    ))
}

fn create_clause(input: &str) -> IResult<&str, Clause> {
    map(
        preceded(
            pair(keyword("CREATE"), multispace0),
            separated_list1(tuple((multispace0, char(','), multispace0)), path_pattern),
        ),
        |patterns| Clause::Create { patterns },
    )(input)
}

fn merge_clause(input: &str) -> IResult<&str, Clause> {
    map(
        preceded(pair(keyword("MERGE"), multispace0), path_pattern),
        |pattern| Clause::Merge { pattern },
    )(input)
}

fn return_clause(input: &str) -> IResult<&str, Clause> {
    let (rest, _) = keyword("RETURN")(input)?;
    let (rest, _) = multispace1(rest)?;
    let (rest, distinct) = opt(terminated(keyword("DISTINCT"), multispace1))(rest)?;
    let (rest, items) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        return_item,
    )(rest)?;
    Ok((
        rest,
        Clause::Return {
            items,
            distinct: distinct.is_some(),
        },
    ))
}

fn with_clause(input: &str) -> IResult<&str, Clause> {
    let (rest, _) = keyword("WITH")(input)?;
    let (rest, _) = multispace1(rest)?;
    let (rest, distinct) = opt(terminated(keyword("DISTINCT"), multispace1))(rest)?;
    let (rest, items) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        return_item,
    )(rest)?;
    let mut rest = rest;
    let mut where_clause = None;
    let mut order_by = Vec::new();
    let mut skip = None;
    let mut limit = None;

    // Parse optional WHERE
    if let Ok((r, wc)) = preceded(multispace1, where_clause)(rest) {
        where_clause = Some(wc);
        rest = r;
    }
    // Parse optional ORDER BY
    if let Ok((r, ob)) = order_by_clause(rest) {
        order_by = ob;
        rest = r.trim_start();
        if let Ok((r, s)) = skip_clause(rest) {
            skip = Some(s);
            rest = r.trim_start();
        }
        if let Ok((r, l)) = limit_clause(rest) {
            limit = Some(l);
            rest = r.trim_start();
        }
    } else {
        // Try SKIP/LIMIT without ORDER BY
        if let Ok((r, s)) = skip_clause(rest) {
            skip = Some(s);
            rest = r.trim_start();
            if let Ok((r, l)) = limit_clause(rest) {
                limit = Some(l);
                rest = r.trim_start();
            }
        }
        if let Ok((r, l)) = limit_clause(rest) {
            limit = Some(l);
            rest = r.trim_start();
        }
    }

    Ok((
        rest,
        Clause::With {
            items,
            distinct: distinct.is_some(),
            where_clause,
            order_by,
            skip,
            limit,
        },
    ))
}

fn delete_clause(input: &str) -> IResult<&str, Clause> {
    // Optional DETACH prefix
    let (rest, detach) = opt(terminated(keyword("DETACH"), multispace1))(input)?;
    let (rest, _) = keyword("DELETE")(rest)?;
    let (rest, _) = multispace1(rest)?;
    let (rest, vars) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        map(identifier, str::to_string),
    )(rest)?;
    Ok((
        rest,
        Clause::Delete {
            variables: vars,
            detach: detach.is_some(),
        },
    ))
}

/// Parse `SET n.prop = value`
fn set_property(input: &str) -> IResult<&str, Clause> {
    let (rest, var) = map(identifier, str::to_string)(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, prop) = preceded(char('.'), map(identifier, str::to_string))(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, _) = char('=')(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, val) = cypher_value(rest)?;
    Ok((
        rest,
        Clause::SetProperty {
            variable: var,
            property: prop,
            value: val,
        },
    ))
}

/// Parse `SET n += {map}`
fn set_merge(input: &str) -> IResult<&str, Clause> {
    let (rest, var) = map(identifier, str::to_string)(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, _) = tag("+=")(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, props) = properties(rest)?;
    Ok((
        rest,
        Clause::SetMerge {
            variable: var,
            properties: props,
        },
    ))
}

/// Parse `SET n:Label:Label2`
fn set_add_labels(input: &str) -> IResult<&str, Clause> {
    let (rest, var) = map(identifier, str::to_string)(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, labels) = many1(preceded(multispace0, label))(rest)?;
    Ok((
        rest,
        Clause::SetAddLabels {
            variable: var,
            labels,
        },
    ))
}

fn set_clause(input: &str) -> IResult<&str, Clause> {
    preceded(
        pair(keyword("SET"), multispace1),
        alt((set_property, set_merge, set_add_labels)),
    )(input)
}

/// Parse `REMOVE n.prop`
fn remove_property(input: &str) -> IResult<&str, Clause> {
    let (rest, var) = map(identifier, str::to_string)(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, prop) = preceded(char('.'), map(identifier, str::to_string))(rest)?;
    Ok((
        rest,
        Clause::RemoveProperty {
            variable: var,
            property: prop,
        },
    ))
}

/// Parse `REMOVE n:Label:Label2`
fn remove_labels(input: &str) -> IResult<&str, Clause> {
    let (rest, var) = map(identifier, str::to_string)(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, labels) = many1(preceded(multispace0, label))(rest)?;
    Ok((
        rest,
        Clause::RemoveLabels {
            variable: var,
            labels,
        },
    ))
}

fn remove_clause(input: &str) -> IResult<&str, Clause> {
    preceded(
        pair(keyword("REMOVE"), multispace1),
        alt((remove_property, remove_labels)),
    )(input)
}

fn clause(input: &str) -> IResult<&str, Clause> {
    alt((
        match_clause,
        create_clause,
        merge_clause,
        return_clause,
        delete_clause,
        set_clause,
        remove_clause,
        with_clause,
    ))(input)
}

/// Parse a single ORDER BY item: `expr [ASC|DESC]`.
fn order_by_item(input: &str) -> IResult<&str, OrderByItem> {
    let (rest, expr) = expression(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, direction) = opt(alt((
        value(OrderDirection::Ascending, keyword("ASC")),
        value(OrderDirection::Descending, keyword("DESC")),
    )))(rest)?;
    Ok((
        rest,
        OrderByItem {
            expression: expr,
            direction: direction.unwrap_or(OrderDirection::Ascending),
        },
    ))
}

/// Parse ORDER BY clause: `ORDER BY expr [ASC|DESC], ...`
fn order_by_clause(input: &str) -> IResult<&str, Vec<OrderByItem>> {
    preceded(
        pair(keyword("ORDER"), multispace1),
        preceded(
            pair(keyword("BY"), multispace0),
            separated_list1(tuple((multispace0, char(','), multispace0)), order_by_item),
        ),
    )(input)
}

/// Parse SKIP clause: `SKIP n`
fn skip_clause(input: &str) -> IResult<&str, u64> {
    map(
        preceded(pair(keyword("SKIP"), multispace1), digit1),
        |s: &str| s.parse().expect("digit1 guarantees valid number"),
    )(input)
}

/// Parse LIMIT clause: `LIMIT n`
fn limit_clause(input: &str) -> IResult<&str, u64> {
    map(
        preceded(pair(keyword("LIMIT"), multispace1), digit1),
        |s: &str| s.parse().expect("digit1 guarantees valid number"),
    )(input)
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Parse a Cypher query string and return the [`CypherQuery`] AST.
///
/// Returns a [`CypherError::ParseError`] if the input could not be fully
/// parsed.
pub(crate) fn parse_query(input: &str) -> Result<CypherQuery, CypherError> {
    let mut remaining = input.trim();
    let mut clauses = Vec::new();

    while !remaining.is_empty() {
        // Try ORDER BY, SKIP, LIMIT before trying regular clauses
        if let Ok((rest, order_by)) = order_by_clause(remaining) {
            let mut skip = None;
            let mut limit = None;
            let mut rest = rest.trim_start();

            if let Ok((r, s)) = skip_clause(rest) {
                skip = Some(s);
                if let Ok((r2, l)) = limit_clause(r.trim_start()) {
                    limit = Some(l);
                    rest = r2.trim_start();
                } else {
                    rest = r.trim_start();
                }
            }

            return Ok(CypherQuery {
                clauses,
                order_by,
                skip,
                limit,
            });
        }

        // Try SKIP or LIMIT standalone (without ORDER BY)
        if let Ok((rest, s)) = skip_clause(remaining) {
            let mut after_skip = rest.trim_start();
            let mut limit = None;
            if let Ok((r, l)) = limit_clause(after_skip) {
                limit = Some(l);
                after_skip = r.trim_start();
            }
            remaining = after_skip;
            return Ok(CypherQuery {
                clauses,
                order_by: vec![],
                skip: Some(s),
                limit,
            });
        }
        if let Ok((rest, l)) = limit_clause(remaining) {
            remaining = rest.trim_start();
            return Ok(CypherQuery {
                clauses,
                order_by: vec![],
                skip: None,
                limit: Some(l),
            });
        }

        let (rest, c) = clause(remaining).map_err(|e| CypherError::ParseError(format!("{}", e)))?;
        clauses.push(c);
        remaining = rest.trim_start();
        // Consume an optional semicolon between/after clauses.
        if remaining.starts_with(';') {
            remaining = remaining[1..].trim_start();
        }
    }

    Ok(CypherQuery {
        clauses,
        order_by: vec![],
        skip: None,
        limit: None,
    })
}
