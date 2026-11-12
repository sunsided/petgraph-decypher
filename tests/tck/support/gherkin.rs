//! A minimal parser for the Gherkin subset the openCypher TCK uses: tags,
//! `Feature`, `Background`, `Scenario`, `Scenario Outline` with `Examples`,
//! steps with doc strings or tables, and comments.

#[derive(Debug, Default)]
pub struct Feature {
    pub tags: Vec<String>,
    pub background: Vec<Step>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Default)]
pub struct Scenario {
    pub name: String,
    /// 1-based line of the `Scenario:` keyword.
    pub line: usize,
    pub tags: Vec<String>,
    pub steps: Vec<Step>,
    pub examples: Vec<Examples>,
}

#[derive(Debug, Default)]
pub struct Examples {
    pub tags: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Step {
    pub text: String,
    pub docstring: Option<String>,
    pub table: Vec<Vec<String>>,
}

const STEP_KEYWORDS: [&str; 5] = ["Given ", "When ", "Then ", "And ", "But "];

enum Target {
    None,
    Background,
    Scenario,
    Examples,
}

pub fn parse(source: &str) -> Result<Feature, String> {
    let mut feature = Feature::default();
    let mut tags = Vec::new();
    let mut target = Target::None;
    let mut lines = source.lines().enumerate();

    while let Some((index, raw)) = lines.next() {
        let line = raw.trim();
        let line_no = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('@') {
            tags.extend(
                line.split_whitespace()
                    .map(|t| t.trim_start_matches('@').to_owned()),
            );
            continue;
        }
        if line.starts_with("Feature:") {
            feature.tags = std::mem::take(&mut tags);
            continue;
        }
        if line.starts_with("Background:") {
            target = Target::Background;
            continue;
        }
        if let Some(name) = line
            .strip_prefix("Scenario Outline:")
            .or_else(|| line.strip_prefix("Scenario:"))
        {
            feature.scenarios.push(Scenario {
                name: name.trim().to_owned(),
                line: line_no,
                tags: std::mem::take(&mut tags),
                ..Scenario::default()
            });
            target = Target::Scenario;
            continue;
        }
        if line.starts_with("Examples:") {
            let scenario = feature
                .scenarios
                .last_mut()
                .ok_or(format!("line {line_no}: Examples outside a scenario"))?;
            scenario.examples.push(Examples {
                tags: std::mem::take(&mut tags),
                rows: Vec::new(),
            });
            target = Target::Examples;
            continue;
        }

        let steps = match target {
            Target::Background => Some(&mut feature.background),
            Target::Scenario => feature.scenarios.last_mut().map(|s| &mut s.steps),
            Target::Examples | Target::None => None,
        };

        if let Some(keyword) = STEP_KEYWORDS.iter().find(|k| line.starts_with(*k)) {
            let steps = steps.ok_or(format!("line {line_no}: step outside a scenario"))?;
            steps.push(Step {
                text: line[keyword.len()..].trim().to_owned(),
                ..Step::default()
            });
            continue;
        }

        if line.starts_with("\"\"\"") {
            let indent = raw.len() - raw.trim_start().len();
            let mut body = Vec::new();
            loop {
                let (_, doc_line) = lines
                    .next()
                    .ok_or(format!("line {line_no}: unterminated doc string"))?;
                if doc_line.trim() == "\"\"\"" {
                    break;
                }
                let strip = doc_line
                    .char_indices()
                    .take_while(|(i, c)| *i < indent && c.is_whitespace())
                    .count();
                body.push(&doc_line[strip..]);
            }
            let step = steps
                .and_then(|s| s.last_mut())
                .ok_or(format!("line {line_no}: doc string without a step"))?;
            step.docstring = Some(body.join("\n"));
            continue;
        }

        if line.starts_with('|') {
            let row = table_row(line);
            match target {
                Target::Examples => {
                    let examples = feature
                        .scenarios
                        .last_mut()
                        .and_then(|s| s.examples.last_mut())
                        .ok_or(format!("line {line_no}: table outside Examples"))?;
                    examples.rows.push(row);
                }
                _ => {
                    let step = steps
                        .and_then(|s| s.last_mut())
                        .ok_or(format!("line {line_no}: table without a step"))?;
                    step.table.push(row);
                }
            }
            continue;
        }

        return Err(format!("line {line_no}: cannot parse `{line}`"));
    }
    Ok(feature)
}

/// Splits `| a | b\|c |` into cells, applying Gherkin's cell escapes
/// (`\|`, `\\`, `\n`).
fn table_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('|') => cell.push('|'),
                Some('\\') => cell.push('\\'),
                Some('n') => cell.push('\n'),
                Some(other) => {
                    cell.push('\\');
                    cell.push(other);
                }
                None => cell.push('\\'),
            },
            '|' => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            c => cell.push(c),
        }
    }
    cells
}
