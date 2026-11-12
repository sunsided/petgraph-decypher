//! Execution conformance gate against the openCypher 2024.3 TCK.
//!
//! Every scenario in `tests/tck/features` is executed against a petgraph
//! graph: setup queries run through the mutation API, the query under test
//! runs, and its result table, expected error and side effects are compared
//! with the scenario (see `tests/tck/README.md`).
//!
//! Known failures are listed in `tests/tck/known_failures.txt`. The test fails
//! on any failure not in the list and on any listed scenario that now passes,
//! so the list can only shrink. Regenerate it with
//! `TCK_BLESS=1 cargo test --test tck`.
//!
//! Environment variables:
//! - `TCK_FILTER=<text>`: run only scenarios whose id or name contains `text`
//!   (the gate is then not enforced);
//! - `TCK_VERBOSE=1`: print every failing scenario with its reason.

#[path = "tck/support/mod.rs"]
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use support::runner::{self, Outcome};

fn tck_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tck")
}

fn feature_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|e| e.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            feature_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "feature") {
            out.push(path);
        }
    }
}

/// Runs `f`, turning a panic into a failure message.
fn guarded(f: impl FnOnce() -> Outcome) -> Outcome {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string payload>".to_owned());
        Outcome::Fail(format!("panic: {message}"))
    })
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

/// Scenario ids listed in `known_failures.txt`.
fn load_known_failures(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| line.split(" # ").next().unwrap_or("").trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// `clauses/match/Match1.feature:12` → `clauses/match`.
fn category(id: &str) -> String {
    let path = id.split(':').next().unwrap_or(id);
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    if parts.is_empty() {
        "(root)".into()
    } else {
        parts.join("/")
    }
}

#[derive(Default)]
struct Tally {
    pass: usize,
    fail: usize,
    skip: usize,
}

fn main() -> ExitCode {
    support::value::self_check();

    let dir = tck_dir();
    let features_dir = dir.join("features");
    let graphs_dir = dir.join("graphs");
    let known_path = dir.join("known_failures.txt");
    let filter = std::env::var("TCK_FILTER").ok().filter(|f| !f.is_empty());
    let verbose = std::env::var_os("TCK_VERBOSE").is_some();
    let bless = std::env::var("TCK_BLESS").is_ok_and(|v| matches!(v.as_str(), "1" | "true"));
    if bless && filter.is_some() {
        eprintln!("TCK_BLESS with TCK_FILTER would drop the known failures outside the filter");
        return ExitCode::FAILURE;
    }

    let mut files = Vec::new();
    feature_files(&features_dir, &mut files);
    let scenarios: Vec<_> = files
        .iter()
        .flat_map(|file| runner::load(&features_dir, file))
        .filter(|s| {
            filter
                .as_deref()
                .is_none_or(|f| s.id.contains(f) || s.name.contains(f))
        })
        .collect();

    // Panics are reported as scenario failures; keep their default output quiet.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let mut skips: BTreeMap<String, usize> = BTreeMap::new();
    for scenario in &scenarios {
        let tally = tallies.entry(category(&scenario.id)).or_default();
        match guarded(|| runner::run(scenario, &graphs_dir)) {
            Outcome::Pass => tally.pass += 1,
            Outcome::Skip(reason) => {
                tally.skip += 1;
                *skips.entry(reason).or_default() += 1;
            }
            Outcome::Fail(reason) => {
                tally.fail += 1;
                failures.insert(scenario.id.clone(), reason);
            }
        }
    }
    std::panic::set_hook(default_hook);

    let mut total = Tally::default();
    println!("\nopenCypher TCK execution results");
    println!(
        "{:<40} {:>6} {:>6} {:>6}",
        "category", "pass", "fail", "skip"
    );
    for (category, t) in &tallies {
        println!("{category:<40} {:>6} {:>6} {:>6}", t.pass, t.fail, t.skip);
        total.pass += t.pass;
        total.fail += t.fail;
        total.skip += t.skip;
    }
    println!(
        "{:<40} {:>6} {:>6} {:>6}\n",
        "total", total.pass, total.fail, total.skip
    );

    for (reason, n) in &skips {
        println!("skipped {n}: {reason}");
    }

    if verbose {
        for (id, reason) in &failures {
            println!("FAIL {id}: {}", one_line(reason, 300));
        }
    }

    if bless {
        let mut text = String::from(
            "# Known openCypher TCK execution failures; see tests/tck.rs.\n\
             # Format: <feature>:<scenario line>[#<example row>] # <reason>\n\
             # Regenerate with: TCK_BLESS=1 cargo test --test tck\n",
        );
        for (id, reason) in &failures {
            text.push_str(&format!("{id} # {}\n", one_line(reason, 160)));
        }
        std::fs::write(&known_path, text).expect("cannot write known_failures.txt");
        println!(
            "wrote {} known failures to {}",
            failures.len(),
            known_path.display()
        );
        return ExitCode::SUCCESS;
    }

    if filter.is_some() {
        println!("TCK_FILTER is set; the known-failures gate is not enforced.");
        return ExitCode::SUCCESS;
    }

    let known = load_known_failures(&known_path);
    let new: Vec<_> = failures.keys().filter(|id| !known.contains(*id)).collect();
    let fixed: Vec<_> = known
        .iter()
        .filter(|id| !failures.contains_key(*id))
        .collect();
    for id in &new {
        println!("NEW FAILURE {id}: {}", one_line(&failures[*id], 300));
    }
    for id in &fixed {
        println!("NOW PASSES {id} (remove it from known_failures.txt)");
    }
    if new.is_empty() && fixed.is_empty() {
        println!("TCK gate passed ({} known failures).", known.len());
        ExitCode::SUCCESS
    } else {
        println!(
            "TCK gate failed: {} new failure(s), {} known failure(s) now pass. \
             Fix them, or rerun with TCK_BLESS=1 after an intended change.",
            new.len(),
            fixed.len()
        );
        ExitCode::FAILURE
    }
}
