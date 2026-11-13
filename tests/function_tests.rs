//! Built-in functions beyond the openCypher TCK, checked against the Neo4j
//! Cypher manual.

use petgraph::Graph;
use petgraph_decypher::{CypherError, EdgeData, NodeData, PetgraphCypher, Value};

/// The single value `RETURN <expr>` produces.
fn value_of(expr: &str) -> Result<Value, CypherError> {
    let g = Graph::<NodeData, EdgeData>::new();
    let result = g.cypher(&format!("RETURN {expr} AS v"))?;
    Ok(result.column("v").next().cloned().unwrap_or_default())
}

fn ok(expr: &str) -> Value {
    value_of(expr).unwrap_or_else(|e| panic!("{expr}: {e}"))
}

fn list<const N: usize>(items: [Value; N]) -> Value {
    Value::List(items.into())
}

#[test]
fn or_null_conversions_give_null_instead_of_failing() {
    // The manual's examples.
    assert_eq!(ok("toBooleanOrNull('true')"), Value::from(true));
    assert_eq!(ok("toBooleanOrNull('not a boolean')"), Value::Null);
    assert_eq!(ok("toBooleanOrNull(0)"), Value::from(false));
    assert_eq!(ok("toBooleanOrNull(1.5)"), Value::Null);
    assert_eq!(ok("toFloatOrNull('11.5')"), Value::from(11.5));
    assert_eq!(ok("toFloatOrNull('not a number')"), Value::Null);
    assert_eq!(ok("toFloatOrNull(true)"), Value::Null);
    assert_eq!(ok("toIntegerOrNull('42')"), Value::from(42));
    assert_eq!(ok("toIntegerOrNull('not a number')"), Value::Null);
    assert_eq!(ok("toIntegerOrNull(true)"), Value::from(1));
    assert_eq!(ok("toIntegerOrNull(['A', 'B', 'C'])"), Value::Null);
    assert_eq!(ok("toStringOrNull(1.5)"), Value::from("1.5"));
    assert_eq!(ok("toStringOrNull([1])"), Value::Null);
    assert_eq!(ok("toStringOrNull(null)"), Value::Null);
}

#[test]
fn list_conversions_convert_each_element_or_null() {
    assert_eq!(
        ok("toIntegerList(['1', 2.7, 'x', null, true])"),
        list([
            Value::from(1),
            Value::from(2),
            Value::Null,
            Value::Null,
            Value::from(1)
        ])
    );
    assert_eq!(
        ok("toFloatList(['1.5', 2, [3]])"),
        list([Value::from(1.5), Value::from(2.0), Value::Null])
    );
    assert_eq!(
        ok("toStringList([1, 'a', {k: 1}])"),
        list([Value::from("1"), Value::from("a"), Value::Null])
    );
    assert_eq!(
        ok("toBooleanList(['true', 0, 'x'])"),
        list([Value::from(true), Value::from(false), Value::Null])
    );
    assert_eq!(ok("toIntegerList(null)"), Value::Null);
    // A non-list argument is an error (decypher already rejects it when
    // analyzing the query).
    assert!(value_of("toIntegerList(1)").is_err());
}

#[test]
fn predicates_and_lengths() {
    assert_eq!(ok("isEmpty([])"), Value::from(true));
    assert_eq!(ok("isEmpty({a: 1})"), Value::from(false));
    assert_eq!(ok("isEmpty('')"), Value::from(true));
    assert_eq!(ok("isEmpty(null)"), Value::Null);
    assert_eq!(ok("isNaN(0 / 0.0)"), Value::from(true));
    assert_eq!(ok("isNaN(1)"), Value::from(false));
    assert_eq!(ok("isNaN(null)"), Value::Null);
    assert_eq!(ok("char_length('Alice')"), Value::from(5));
    assert_eq!(ok("character_length('é😀')"), Value::from(2));
    assert_eq!(ok("char_length(null)"), Value::Null);
}

#[test]
fn null_if_and_aliases() {
    assert_eq!(ok("nullIf(4, 4)"), Value::Null);
    assert_eq!(ok("nullIf('abc', 'def')"), Value::from("abc"));
    assert_eq!(ok("nullIf(1, null)"), Value::from(1));
    assert_eq!(ok("lower('AbC')"), Value::from("abc"));
    assert_eq!(ok("upper('AbC')"), Value::from("ABC"));
}

#[test]
fn trims_take_an_optional_trim_character() {
    assert_eq!(ok("btrim('  a  ')"), Value::from("a"));
    assert_eq!(ok("btrim('xxaxx', 'x')"), Value::from("a"));
    assert_eq!(ok("ltrim('xxaxx', 'x')"), Value::from("axx"));
    assert_eq!(ok("rtrim('xxaxx', 'x')"), Value::from("xxa"));
    assert_eq!(ok("ltrim('  a  ')"), Value::from("a  "));
    assert_eq!(ok("btrim(null, 'x')"), Value::Null);
    assert!(matches!(
        value_of("btrim('xxaxx', 'xx')"),
        Err(CypherError::RuntimeError(_))
    ));
}

#[test]
fn split_on_any_of_several_delimiters() {
    let parts = |items: &[&str]| Value::List(items.iter().map(|s| Value::from(*s)).collect());
    assert_eq!(ok("split('a-b_c', ['-', '_'])"), parts(&["a", "b", "c"]));
    assert_eq!(ok("split('a--b', ['-', '--'])"), parts(&["a", "b"]));
    assert_eq!(ok("split('é-ü', ['-'])"), parts(&["é", "ü"]));
    assert_eq!(ok("split('a-b', ['-', null])"), Value::Null);
    assert_eq!(ok("split('a-b', '-')"), parts(&["a", "b"]));
}

#[test]
fn variance_aggregates() {
    let g = Graph::<NodeData, EdgeData>::new();
    let r = g
        .cypher(
            "UNWIND [2, 4, 4, 4, 5, 5, 7, 9] AS x RETURN var(x) AS s, varP(x) AS p, stDevP(x) AS d",
        )
        .unwrap();
    let row = r.rows().next().unwrap();
    assert_eq!(row.get("s"), Some(&Value::from(32.0 / 7.0)));
    assert_eq!(row.get("p"), Some(&Value::from(4.0)));
    assert_eq!(row.get("d"), Some(&Value::from(2.0)));
    let r = g.cypher("UNWIND [1] AS x RETURN var(x) AS s").unwrap();
    assert_eq!(r.column("s").next(), Some(&Value::from(0.0)));
}
