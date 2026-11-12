//! Expands TCK feature files into scenarios and interprets their steps.

use std::collections::BTreeMap;
use std::path::Path;

use super::engine::{Engine, ExecError, Snapshot, Table};
use super::gherkin::{self, Step};
use super::value::{self, TckValue};

/// One executable scenario; outlines yield one per example row.
pub struct Scenario {
    /// `<feature path relative to features/>:<line>`, plus `#<n>` for the
    /// n-th example row of an outline.
    pub id: String,
    pub name: String,
    steps: Vec<Step>,
    ignored: bool,
}

pub enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

/// Parses a feature file and expands its scenarios.
pub fn load(features_dir: &Path, path: &Path) -> Vec<Scenario> {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let feature =
        gherkin::parse(&source).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()));
    let rel = path
        .strip_prefix(features_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let feature_ignored = is_ignored(&feature.tags);

    let mut out = Vec::new();
    for scenario in &feature.scenarios {
        let id = format!("{rel}:{}", scenario.line);
        let ignored = feature_ignored || is_ignored(&scenario.tags);
        let base: Vec<Step> = feature
            .background
            .iter()
            .chain(&scenario.steps)
            .cloned()
            .collect();
        if scenario.examples.is_empty() {
            out.push(Scenario {
                id,
                name: scenario.name.clone(),
                steps: base,
                ignored,
            });
            continue;
        }
        let mut n = 0;
        for examples in &scenario.examples {
            let Some((header, rows)) = examples.rows.split_first() else {
                continue;
            };
            for row in rows {
                n += 1;
                let subst = |s: &str| {
                    header.iter().zip(row).fold(s.to_owned(), |acc, (k, v)| {
                        acc.replace(&format!("<{k}>"), v)
                    })
                };
                let steps = base
                    .iter()
                    .map(|step| Step {
                        text: subst(&step.text),
                        docstring: step.docstring.as_deref().map(subst),
                        table: step
                            .table
                            .iter()
                            .map(|r| r.iter().map(|c| subst(c)).collect())
                            .collect(),
                    })
                    .collect();
                out.push(Scenario {
                    id: format!("{id}#{n}"),
                    name: subst(&scenario.name),
                    steps,
                    ignored: ignored || is_ignored(&examples.tags),
                });
            }
        }
    }
    out
}

fn is_ignored(tags: &[String]) -> bool {
    tags.iter().any(|t| t == "ignore")
}

/// Runs one scenario.
pub fn run(scenario: &Scenario, graphs_dir: &Path) -> Outcome {
    if scenario.ignored {
        return Outcome::Skip("tagged @ignore".into());
    }
    let mut state = State {
        engine: Engine::default(),
        params: BTreeMap::new(),
        result: None,
        before: None,
    };
    for step in &scenario.steps {
        match state.step(step, graphs_dir) {
            Ok(()) => {}
            Err(StepError::Fail(m)) => return Outcome::Fail(m),
            Err(StepError::Skip(m)) => return Outcome::Skip(m),
        }
    }
    Outcome::Pass
}

enum StepError {
    Fail(String),
    Skip(String),
}

fn fail<T>(message: impl Into<String>) -> Result<T, StepError> {
    Err(StepError::Fail(message.into()))
}

struct State {
    engine: Engine,
    params: BTreeMap<String, TckValue>,
    result: Option<Result<Table, ExecError>>,
    before: Option<Snapshot>,
}

impl State {
    fn step(&mut self, step: &Step, graphs_dir: &Path) -> Result<(), StepError> {
        let text = step.text.as_str();
        let docstring = || step.docstring.as_deref().unwrap_or("");
        let table = || step.table.as_slice();

        if text == "an empty graph" || text == "any graph" {
            return Ok(());
        }
        if let Some(name) = text
            .strip_prefix("the ")
            .and_then(|t| t.strip_suffix(" graph"))
        {
            let script = graphs_dir.join(name).join(format!("{name}.cypher"));
            let query = std::fs::read_to_string(&script)
                .map_err(|e| StepError::Fail(format!("named graph {name}: {e}")))?;
            return self.setup(&query);
        }
        if text == "having executed:" {
            return self.setup(docstring());
        }
        if text == "parameters are:" {
            for row in table() {
                let [name, cell] = row.as_slice() else {
                    return fail(format!("malformed parameter row {row:?}"));
                };
                let value = value::parse(cell).map_err(StepError::Fail)?;
                self.params.insert(name.clone(), value);
            }
            return Ok(());
        }
        if text.starts_with("there exists a procedure") {
            return Err(StepError::Skip("needs a test procedure".into()));
        }
        if text == "executing query:" || text == "executing control query:" {
            self.before = Some(self.engine.snapshot());
            self.result = Some(self.engine.execute(docstring(), &self.params));
            return Ok(());
        }
        if text == "the result should be empty" {
            let table = self.table()?;
            if !table.rows.is_empty() {
                return fail(format!("expected no rows, got {}", table.rows.len()));
            }
            return Ok(());
        }
        if let Some(mode) = text.strip_prefix("the result should be") {
            let ordered = mode.contains("in order");
            let sort_lists = mode.contains("ignoring element order for lists");
            return self.compare(table(), ordered, sort_lists);
        }
        if text.starts_with("no side effects") {
            return self.side_effects(&[]);
        }
        if text == "the side effects should be:" {
            return self.side_effects(table());
        }
        if let Some(rest) = text.strip_prefix("a ").or_else(|| text.strip_prefix("an "))
            && let Some((kind, _)) = rest.split_once(" should be raised")
        {
            return self.expect_error(kind, rest);
        }
        fail(format!("unknown step `{text}`"))
    }

    fn setup(&mut self, query: &str) -> Result<(), StepError> {
        self.engine
            .setup(query)
            .map_err(|e| StepError::Fail(format!("setup failed: {e}")))
    }

    fn table(&self) -> Result<&Table, StepError> {
        match &self.result {
            Some(Ok(table)) => Ok(table),
            Some(Err(e)) => fail(e.to_string()),
            None => fail("no query was executed"),
        }
    }

    fn compare(
        &self,
        expected: &[Vec<String>],
        ordered: bool,
        sort_lists: bool,
    ) -> Result<(), StepError> {
        let table = self.table()?;
        let Some((header, rows)) = expected.split_first() else {
            return fail("expected result table without header");
        };

        let mut actual_columns = table.columns.clone();
        let mut expected_columns = header.clone();
        actual_columns.sort();
        expected_columns.sort();
        if actual_columns != expected_columns {
            return fail(format!(
                "columns differ: expected {header:?}, got {:?}",
                table.columns
            ));
        }

        let mut want = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| value::parse(cell).map(|v| v.canonical(sort_lists)))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| StepError::Fail(format!("harness: {e}")))?;
        let mut got: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|row| {
                header
                    .iter()
                    .map(|col| {
                        row.get(col)
                            .map_or_else(|| "<missing>".into(), |v| v.canonical(sort_lists))
                    })
                    .collect()
            })
            .collect();
        if !ordered {
            want.sort();
            got.sort();
        }
        if want != got {
            return fail(format!(
                "rows differ: expected {}, got {}",
                render_rows(&want),
                render_rows(&got)
            ));
        }
        Ok(())
    }

    fn side_effects(&mut self, expected: &[Vec<String>]) -> Result<(), StepError> {
        self.table()?;
        let Some(before) = self.before.take() else {
            return fail("no query was executed");
        };
        let after = self.engine.snapshot();
        let count = |a: usize| a as i64;
        let actual = [
            (
                "+nodes",
                count(after.nodes.difference(&before.nodes).count()),
            ),
            (
                "-nodes",
                count(before.nodes.difference(&after.nodes).count()),
            ),
            (
                "+relationships",
                count(after.rels.difference(&before.rels).count()),
            ),
            (
                "-relationships",
                count(before.rels.difference(&after.rels).count()),
            ),
            (
                "+labels",
                count(after.labels.difference(&before.labels).count()),
            ),
            (
                "-labels",
                count(before.labels.difference(&after.labels).count()),
            ),
            (
                "+properties",
                count(after.properties.difference(&before.properties).count()),
            ),
            (
                "-properties",
                count(before.properties.difference(&after.properties).count()),
            ),
        ];
        let mut want: BTreeMap<&str, i64> = BTreeMap::new();
        for row in expected {
            let [key, n] = row.as_slice() else {
                return fail(format!("malformed side effect row {row:?}"));
            };
            let n = n
                .trim()
                .parse()
                .map_err(|_| StepError::Fail(format!("bad side effect count {n:?}")))?;
            want.insert(key.trim(), n);
        }
        let diffs: Vec<String> = actual
            .iter()
            .filter_map(|(key, got)| {
                let want = want.get(key).copied().unwrap_or(0);
                (want != *got).then(|| format!("{key}: expected {want}, got {got}"))
            })
            .collect();
        if diffs.is_empty() {
            Ok(())
        } else {
            fail(format!("side effects differ: {}", diffs.join(", ")))
        }
    }

    fn expect_error(&self, kind: &str, description: &str) -> Result<(), StepError> {
        match &self.result {
            Some(Err(ExecError::Failed(_))) => Ok(()),
            Some(Err(e @ ExecError::Unsupported(_))) => fail(format!("expected {kind}, got {e}")),
            Some(Ok(table)) => fail(format!(
                "expected {description}, got {} row(s)",
                table.rows.len()
            )),
            None => fail("no query was executed"),
        }
    }
}

fn render_rows(rows: &[Vec<String>]) -> String {
    let rendered: Vec<String> = rows
        .iter()
        .map(|r| format!("[{}]", r.join(" | ")))
        .collect();
    format!("{{{}}}", rendered.join(", "))
}
