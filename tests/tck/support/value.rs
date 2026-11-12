//! TCK result values: a parser for the value syntax used in expected result
//! tables, and a canonical rendering that both expected and actual values are
//! compared by.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A value as it appears in a TCK result table.
#[derive(Debug, Clone)]
pub enum TckValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<TckValue>),
    Map(BTreeMap<String, TckValue>),
    Node(Node),
    Rel(Rel),
    Path(Path),
}

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub labels: Vec<String>,
    pub properties: BTreeMap<String, TckValue>,
}

#[derive(Debug, Clone, Default)]
pub struct Rel {
    pub rel_type: String,
    pub properties: BTreeMap<String, TckValue>,
}

#[derive(Debug, Clone)]
pub struct Path {
    pub start: Node,
    /// `(relationship, points forward, next node)` per hop.
    pub hops: Vec<(Rel, bool, Node)>,
}

impl TckValue {
    /// Renders the value canonically: map keys and labels sorted, floats always
    /// with a fractional part. With `sort_lists`, list elements are sorted too,
    /// implementing the TCK's "ignoring element order for lists".
    pub fn canonical(&self, sort_lists: bool) -> String {
        let mut out = String::new();
        self.render(&mut out, sort_lists);
        out
    }

    fn render(&self, out: &mut String, sort_lists: bool) {
        match self {
            TckValue::Null => out.push_str("null"),
            TckValue::Bool(b) => write!(out, "{b}").unwrap(),
            TckValue::Int(i) => write!(out, "{i}").unwrap(),
            TckValue::Float(f) => render_float(out, *f),
            TckValue::String(s) => {
                write!(out, "'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")).unwrap()
            }
            TckValue::List(items) => {
                let mut rendered: Vec<String> =
                    items.iter().map(|v| v.canonical(sort_lists)).collect();
                if sort_lists {
                    rendered.sort();
                }
                write!(out, "[{}]", rendered.join(", ")).unwrap();
            }
            TckValue::Map(map) => render_map(out, map, sort_lists),
            TckValue::Node(node) => render_node(out, node, sort_lists),
            TckValue::Rel(rel) => render_rel(out, rel, sort_lists),
            TckValue::Path(path) => {
                out.push('<');
                render_node(out, &path.start, sort_lists);
                for (rel, forward, node) in &path.hops {
                    out.push_str(if *forward { "-" } else { "<-" });
                    render_rel(out, rel, sort_lists);
                    out.push_str(if *forward { "->" } else { "-" });
                    render_node(out, node, sort_lists);
                }
                out.push('>');
            }
        }
    }
}

fn render_float(out: &mut String, f: f64) {
    if f.is_nan() {
        out.push_str("NaN");
    } else if f.is_infinite() {
        out.push_str(if f > 0.0 { "Inf" } else { "-Inf" });
    } else {
        // `{:?}` keeps a fractional part (`1.0`) and round-trips exactly;
        // `-0.0` equals `0.0` in Cypher.
        let f = if f == 0.0 { 0.0 } else { f };
        write!(out, "{f:?}").unwrap();
    }
}

fn render_map(out: &mut String, map: &BTreeMap<String, TckValue>, sort_lists: bool) {
    out.push('{');
    for (i, (k, v)) in map.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write!(out, "{k}: ").unwrap();
        v.render(out, sort_lists);
    }
    out.push('}');
}

fn render_props(out: &mut String, props: &BTreeMap<String, TckValue>, sort_lists: bool) {
    if !props.is_empty() {
        out.push(' ');
        render_map(out, props, sort_lists);
    }
}

fn render_node(out: &mut String, node: &Node, sort_lists: bool) {
    let mut labels = node.labels.clone();
    labels.sort();
    out.push('(');
    for label in &labels {
        write!(out, ":{label}").unwrap();
    }
    render_props(out, &node.properties, sort_lists);
    out.push(')');
}

fn render_rel(out: &mut String, rel: &Rel, sort_lists: bool) {
    write!(out, "[:{}", rel.rel_type).unwrap();
    render_props(out, &rel.properties, sort_lists);
    out.push(']');
}

/// Parses one cell of a TCK table.
pub fn parse(input: &str) -> Result<TckValue, String> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
        src: input,
    };
    let value = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing input in TCK value `{input}` at {}", p.i));
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    src: &'a str,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, token: &str) -> bool {
        self.ws();
        if self.src[self.i..].starts_with(token) {
            self.i += token.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &str) -> Result<(), String> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(format!(
                "expected `{token}` at {} in `{}`",
                self.i, self.src
            ))
        }
    }

    fn value(&mut self) -> Result<TckValue, String> {
        match self.peek() {
            Some(b'\'') => self.string().map(TckValue::String),
            Some(b'[') => {
                // A list, unless it is a relationship `[:T ...]`.
                let save = self.i;
                self.i += 1;
                if self.peek() == Some(b':') {
                    self.i = save;
                    return self.rel().map(TckValue::Rel);
                }
                let mut items = Vec::new();
                if !self.eat("]") {
                    loop {
                        items.push(self.value()?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                Ok(TckValue::List(items))
            }
            Some(b'{') => self.map().map(TckValue::Map),
            Some(b'(') => self.node().map(TckValue::Node),
            Some(b'<') => self.path().map(TckValue::Path),
            Some(_) => self.atom(),
            None => Err(format!("unexpected end of TCK value `{}`", self.src)),
        }
    }

    fn atom(&mut self) -> Result<TckValue, String> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || b"+-._".contains(&self.s[self.i]))
        {
            self.i += 1;
        }
        let word = &self.src[start..self.i];
        Ok(match word {
            "null" => TckValue::Null,
            "true" => TckValue::Bool(true),
            "false" => TckValue::Bool(false),
            "NaN" => TckValue::Float(f64::NAN),
            "Inf" | "Infinity" => TckValue::Float(f64::INFINITY),
            "-Inf" | "-Infinity" => TckValue::Float(f64::NEG_INFINITY),
            _ => {
                if let Ok(i) = word.parse::<i64>() {
                    TckValue::Int(i)
                } else if let Ok(f) = word.parse::<f64>() {
                    TckValue::Float(f)
                } else {
                    return Err(format!("cannot parse TCK value `{word}` in `{}`", self.src));
                }
            }
        })
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect("'")?;
        let mut out = String::new();
        let mut chars = self.src[self.i..].char_indices();
        while let Some((off, c)) = chars.next() {
            match c {
                '\'' => {
                    self.i += off + 1;
                    return Ok(out);
                }
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, other)) => out.push(other),
                    None => break,
                },
                c => out.push(c),
            }
        }
        Err(format!("unterminated string in `{}`", self.src))
    }

    fn name(&mut self) -> Result<String, String> {
        self.ws();
        if self.eat("`") {
            let end = self.src[self.i..]
                .find('`')
                .ok_or_else(|| format!("unterminated name in `{}`", self.src))?;
            let name = self.src[self.i..self.i + end].to_owned();
            self.i += end + 1;
            return Ok(name);
        }
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(format!("expected a name at {} in `{}`", self.i, self.src));
        }
        Ok(self.src[start..self.i].to_owned())
    }

    fn map(&mut self) -> Result<BTreeMap<String, TckValue>, String> {
        self.expect("{")?;
        let mut map = BTreeMap::new();
        if self.eat("}") {
            return Ok(map);
        }
        loop {
            let key = self.name()?;
            self.expect(":")?;
            map.insert(key, self.value()?);
            if self.eat("}") {
                return Ok(map);
            }
            self.expect(",")?;
        }
    }

    fn node(&mut self) -> Result<Node, String> {
        self.expect("(")?;
        let mut node = Node::default();
        while self.eat(":") {
            node.labels.push(self.name()?);
        }
        if self.peek() == Some(b'{') {
            node.properties = self.map()?;
        }
        self.expect(")")?;
        Ok(node)
    }

    fn rel(&mut self) -> Result<Rel, String> {
        self.expect("[")?;
        self.expect(":")?;
        let mut rel = Rel {
            rel_type: self.name()?,
            ..Rel::default()
        };
        if self.peek() == Some(b'{') {
            rel.properties = self.map()?;
        }
        self.expect("]")?;
        Ok(rel)
    }

    fn path(&mut self) -> Result<Path, String> {
        self.expect("<")?;
        let start = self.node()?;
        let mut hops = Vec::new();
        while !self.eat(">") {
            let backward = self.eat("<-");
            if !backward {
                self.expect("-")?;
            }
            let rel = self.rel()?;
            let forward = if backward {
                self.expect("-")?;
                false
            } else {
                self.expect("->")?;
                true
            };
            hops.push((rel, forward, self.node()?));
        }
        Ok(Path { start, hops })
    }
}

/// Self-check of the parser and renderer, run once at the start of the suite.
pub fn self_check() {
    let cases = [
        ("null", "null"),
        ("-1", "-1"),
        ("1.0", "1.0"),
        ("1e3", "1000.0"),
        ("'it\\'s'", "'it\\'s'"),
        ("[1, 'a', [true]]", "[1, 'a', [true]]"),
        ("{b: 2, a: 1}", "{a: 1, b: 2}"),
        ("()", "()"),
        ("(:B:A {name: 'x'})", "(:A:B {name: 'x'})"),
        ("[:T {k: 1}]", "[:T {k: 1}]"),
        ("<(:A)-[:T]->(:B)<-[:U]-()>", "<(:A)-[:T]->(:B)<-[:U]-()>"),
        ("[(:A), [:T]]", "[(:A), [:T]]"),
    ];
    for (input, expected) in cases {
        let actual = parse(input)
            .unwrap_or_else(|e| panic!("{e}"))
            .canonical(false);
        assert_eq!(actual, expected, "canonical form of `{input}`");
    }
    let unordered = parse("[3, 1, 2]").unwrap().canonical(true);
    assert_eq!(unordered, "[1, 2, 3]");
}
