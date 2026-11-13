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

#[test]
fn element_ids_are_distinct_strings_for_nodes_and_relationships() {
    let g = petgraph_decypher::build_graph_from_cypher("CREATE (:A)-[:R]->(:B)").unwrap();
    let r = g
        .cypher(
            "MATCH (a:A)-[r]->(b) RETURN elementId(a) AS a, elementId(r) AS r, elementId(b) AS b",
        )
        .unwrap();
    let values: Vec<Value> = r.rows().next().unwrap().values().to_vec();
    // Node 0 and relationship 0 share an index but not an element id.
    assert_eq!(
        values,
        [
            Value::from("node:0"),
            Value::from("relationship:0"),
            Value::from("node:1")
        ]
    );
    let found = g
        .cypher_params(
            "MATCH (n) WHERE elementId(n) = $id RETURN count(n) AS c",
            &petgraph_decypher::Parameters::from([("id".to_owned(), Value::from("node:1"))]),
        )
        .unwrap();
    assert_eq!(found.column("c").next(), Some(&Value::from(1)));
    assert_eq!(ok("elementId(null)"), Value::Null);
}

#[test]
fn random_uuids_are_version_4_and_differ() {
    let g = Graph::<NodeData, EdgeData>::new();
    let r = g
        .cypher("UNWIND range(1, 50) AS i RETURN randomUUID() AS u")
        .unwrap();
    let uuids: Vec<String> = r
        .column("u")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => panic!("not a string: {other:?}"),
        })
        .collect();
    for u in &uuids {
        let groups: Vec<&str> = u.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "{u}"
        );
        assert!(
            u.chars()
                .all(|c| c == '-' || matches!(c, '0'..='9' | 'a'..='f')),
            "{u}"
        );
        assert!(groups[2].starts_with('4'), "version: {u}");
        assert!(
            matches!(groups[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'),
            "variant: {u}"
        );
    }
    let unique: std::collections::BTreeSet<_> = uuids.iter().collect();
    assert_eq!(unique.len(), uuids.len());
}

#[test]
fn timestamp_is_fixed_for_the_whole_query() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let g = Graph::<NodeData, EdgeData>::new();
    let r = g
        .cypher("UNWIND range(1, 2000) AS i RETURN count(DISTINCT timestamp()) AS n, max(timestamp()) AS t")
        .unwrap();
    let row = r.rows().next().unwrap();
    assert_eq!(row.get("n"), Some(&Value::from(1)));
    let Some(Value::Integer(t)) = row.get("t") else {
        panic!("no timestamp")
    };
    assert!((before..before + 60_000).contains(t), "{t} vs {before}");
}

#[test]
#[allow(clippy::approx_constant)] // 3.142 is the manual's expected result, not pi
fn round_follows_the_manual_examples() {
    let f = |expr: &str| match ok(expr) {
        Value::Float(x) => x,
        other => panic!("{expr}: {other:?}"),
    };
    assert_eq!(f("round(3.141592)"), 3.0);
    assert_eq!(f("round(-1.5)"), -1.0);
    assert_eq!(f("round(3.141592, 3)"), 3.142);
    assert_eq!(f("round(-1.5, 0)"), -1.0);
    assert_eq!(f("round(-1.55, 1)"), -1.6);
    // Modes on 1.249, -1.251, 1.25, -1.35 with precision 1.
    for (mode, expected) in [
        ("UP", [1.3, -1.3, 1.3, -1.4]),
        ("DOWN", [1.2, -1.2, 1.2, -1.3]),
        ("CEILING", [1.3, -1.2, 1.3, -1.3]),
        ("FLOOR", [1.2, -1.3, 1.2, -1.4]),
        ("HALF_UP", [1.2, -1.3, 1.3, -1.4]),
        ("HALF_DOWN", [1.2, -1.3, 1.2, -1.3]),
        ("HALF_EVEN", [1.2, -1.3, 1.2, -1.4]),
    ] {
        let got =
            ["1.249", "-1.251", "1.25", "-1.35"].map(|x| f(&format!("round({x}, 1, '{mode}')")));
        assert_eq!(got, expected, "{mode}");
    }
}

#[test]
fn round_with_precision_edge_cases() {
    let f = |expr: &str| match ok(expr) {
        Value::Float(x) => x,
        other => panic!("{expr}: {other:?}"),
    };
    // Rounds the decimal form, not the slightly smaller double.
    assert_eq!(f("round(2.675, 2)"), 2.68);
    // Negative precision rounds to tens, hundreds, …
    assert_eq!(f("round(1234, -2)"), 1200.0);
    assert_eq!(f("round(1250, -2, 'HALF_EVEN')"), 1200.0);
    assert_eq!(f("round(1350, -2, 'HALF_EVEN')"), 1400.0);
    // Digits far below the precision.
    assert_eq!(f("round(0.0004, 2, 'UP')"), 0.01);
    assert_eq!(f("round(0.0004, 2)"), 0.0);
    assert_eq!(f("round(-0.4, 0, 'DOWN')"), 0.0);
    assert_eq!(f("round(9.99, 1)"), 10.0);
    // Nothing to drop.
    assert_eq!(f("round(1.5, 5)"), 1.5);
    assert_eq!(f("round(1e300, 2)"), 1e300);
    assert!(f("round(0.0 / 0.0, 2)").is_nan());
    assert_eq!(f("round(1, 1)"), 1.0);
    assert_eq!(ok("round(null, 1)"), Value::Null);
    assert_eq!(ok("round(1.5, null)"), Value::Null);
    assert!(matches!(
        value_of("round(1.5, 1, 'SIDEWAYS')"),
        Err(CypherError::RuntimeError(_))
    ));
}
