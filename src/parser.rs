//! nom-based parser for a useful subset of the OpenCypher query language.

use std::collections::HashMap;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_until, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt, recognize, value},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, terminated, tuple},
    IResult,
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

/// Parse a relationship type prefixed with `:` (e.g. `:KNOWS`).
fn rel_type_spec(input: &str) -> IResult<&str, String> {
    map(preceded(char(':'), identifier), str::to_string)(input)
}

/// The inner components of a relationship bracket: `(variable, type, properties)`.
type RelInner = (Option<String>, Option<String>, HashMap<String, CypherValue>);

/// Parse the inner part of `[var:TYPE {props}]`.
fn rel_inner(input: &str) -> IResult<&str, RelInner> {
    map(
        tuple((
            opt(map(identifier, str::to_string)),
            opt(preceded(multispace0, rel_type_spec)),
            opt(preceded(multispace0, properties)),
        )),
        |(var, rtype, props)| (var, rtype, props.unwrap_or_default()),
    )(input)
}

/// `-[...]->`  or  `-[...]-`
fn rel_right_or_undir(input: &str) -> IResult<&str, RelPattern> {
    let (rest, _) = tag("-[")(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, (var, rtype, props)) = rel_inner(rest)?;
    let (rest, _) = multispace0(rest)?;
    // Try `]->` first, then `]-`
    let (rest, direction) = alt((
        value(RelDirection::Right, tag("]->")),
        value(RelDirection::Both, tag("]-")),
    ))(rest)?;
    Ok((
        rest,
        RelPattern {
            variable: var,
            rel_type: rtype,
            properties: props,
            direction,
        },
    ))
}

/// `<-[...]-`
fn rel_left_bracketed(input: &str) -> IResult<&str, RelPattern> {
    let (rest, _) = tag("<-[")(input)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, (var, rtype, props)) = rel_inner(rest)?;
    let (rest, _) = multispace0(rest)?;
    let (rest, _) = tag("]-")(rest)?;
    Ok((
        rest,
        RelPattern {
            variable: var,
            rel_type: rtype,
            properties: props,
            direction: RelDirection::Left,
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

/// Parse an expression: wildcard `*`, variable reference, or `var.prop` property access.
fn expression(input: &str) -> IResult<&str, Expression> {
    alt((
        value(Expression::All, char('*')),
        // var.prop
        map(
            pair(
                map(identifier, str::to_string),
                opt(preceded(char('.'), map(identifier, str::to_string))),
            ),
            |(var, prop)| match prop {
                Some(p) => Expression::Property(var, p),
                None => Expression::Variable(var),
            },
        ),
    ))(input)
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

/// Parse an equality expression `expr = value` inside a WHERE clause.
fn where_equality(input: &str) -> IResult<&str, WhereExpr> {
    map(
        tuple((
            preceded(multispace0, where_expression),
            preceded(multispace0, char('=')),
            preceded(multispace0, cypher_value),
        )),
        |(expr, _, val)| WhereExpr::Eq(expr, val),
    )(input)
}

/// Parse a WHERE expression: only `var.prop` property access is supported.
/// Bare variables and wildcards are rejected to prevent silent "match nothing" behavior.
fn where_expression(input: &str) -> IResult<&str, Expression> {
    map(
        pair(
            map(identifier, str::to_string),
            preceded(char('.'), map(identifier, str::to_string)),
        ),
        |(var, prop)| Expression::Property(var, prop),
    )(input)
}

/// Parse a WHERE expression, supporting chained `AND`-connected equality checks.
fn where_expr(input: &str) -> IResult<&str, WhereExpr> {
    // Only equality for now; AND / OR can be layered on top
    let (rest, first) = where_equality(input)?;
    let (rest, rest_exprs) = many0(preceded(
        tuple((multispace1, keyword("AND"), multispace1)),
        where_equality,
    ))(rest)?;

    let combined = rest_exprs
        .into_iter()
        .fold(first, |acc, e| WhereExpr::And(Box::new(acc), Box::new(e)));
    Ok((rest, combined))
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
    map(
        preceded(
            pair(keyword("RETURN"), multispace1),
            separated_list1(tuple((multispace0, char(','), multispace0)), return_item),
        ),
        |items| Clause::Return { items },
    )(input)
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

fn clause(input: &str) -> IResult<&str, Clause> {
    alt((
        match_clause,
        create_clause,
        merge_clause,
        return_clause,
        delete_clause,
    ))(input)
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
        let (rest, c) = clause(remaining).map_err(|e| CypherError::ParseError(format!("{}", e)))?;
        clauses.push(c);
        remaining = rest.trim_start();
        // Consume an optional semicolon between/after clauses.
        if remaining.starts_with(';') {
            remaining = remaining[1..].trim_start();
        }
    }

    Ok(CypherQuery { clauses })
}
