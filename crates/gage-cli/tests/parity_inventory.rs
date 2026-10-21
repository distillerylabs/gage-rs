//! Every Go test function in `tests/fixtures/go/test-inventory.tsv` is
//! ported or excused, and stays that way (Q-INVENTORY-GUARD).
//!
//! A row passes in one of two ways:
//!
//! - **Name match.** The Go name with `Test` dropped, lowercased, equals a
//!   Rust `#[test]` fn name with `_` removed and lowercased
//!   (`TestEditYAMLScratchFileCreated0600` matches
//!   `edit_yaml_scratch_file_created_0600`).
//! - **An exception.** A row in `tests/parity/inventory-exceptions.tsv`,
//!   either `ported` (naming the Rust tests that port it under another
//!   name) or `excused` (with a `ref` saying why it has no counterpart).
//!   A row that already name-matches may not appear there, so the file is
//!   exactly the set of judgement calls.
//!
//! The Rust side is collected by reading `crates/**/*.rs` with `std::fs`,
//! not by asking the test harness: platform-cfg'd tests are real ports that
//! run on the OS they apply to, and must count on every OS. This file lives
//! in `gage-cli` because it reads both crates' sources.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const INVENTORY: &str = "tests/fixtures/go/test-inventory.tsv";
const EXCEPTIONS: &str = "tests/parity/inventory-exceptions.tsv";
const EXCEPTIONS_HEADER: &str = "go_file\tgo_test\tdisposition\trust_tests\tref";
/// Where an exception's `ref` points: a `Q-` entry here, or a doc path
/// relative to this directory.
const PLANS: &str = "doc/implementation/00_gage-cli/plans/rust-port";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// `TestFooBar` -> `foobar`.
fn normalize_go(name: &str) -> String {
    name.strip_prefix("Test").unwrap_or(name).to_lowercase()
}

/// `foo_bar` -> `foobar`.
fn normalize_rust(name: &str) -> String {
    name.replace('_', "").to_lowercase()
}

/// The names of the `#[test]` fns in one Rust source file. Other attribute
/// lines (`#[cfg(...)]`, `#[ignore]`, ...), comments and blank lines may sit
/// between the `#[test]` and its `fn`.
fn test_fns(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in source.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#[test]") {
            pending = true;
            // `#[test] fn name()` on one line.
            if let Some(name) = fn_name(rest.trim()) {
                names.push(name);
                pending = false;
            }
            continue;
        }
        if !pending {
            continue;
        }
        if let Some(name) = fn_name(line) {
            names.push(name);
            pending = false;
        } else if !(line.is_empty() || line.starts_with("#[") || line.starts_with("//")) {
            pending = false;
        }
    }
    names
}

/// The fn name a line declares, if it starts a fn item.
fn fn_name(line: &str) -> Option<String> {
    let mut rest = line;
    for qualifier in ["pub ", "async ", "unsafe "] {
        rest = rest.strip_prefix(qualifier).unwrap_or(rest).trim_start();
    }
    let rest = rest.strip_prefix("fn ")?.trim_start();
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Every `#[test]` fn name under `crates/`, as written.
fn rust_tests() -> BTreeSet<String> {
    fn walk(dir: &Path, names: &mut BTreeSet<String>) {
        let entries =
            fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&path, names);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                names.extend(test_fns(&source));
            }
        }
    }
    let mut names = BTreeSet::new();
    walk(&repo_root().join("crates"), &mut names);
    // Guards against a scan that silently finds nothing (a moved tree, a
    // changed attribute style) and so turns every check below vacuous.
    assert!(
        names.len() > 1000,
        "found only {} Rust tests under crates/",
        names.len()
    );
    names
}

#[derive(Debug)]
struct InventoryRow {
    phase: String,
    file: String,
    test: String,
}

fn parse_inventory(source: &str) -> Vec<InventoryRow> {
    let mut lines = source.lines();
    assert_eq!(lines.next(), Some("phase\tfile\ttest"), "inventory header");
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 3, "inventory row {l:?}");
            InventoryRow {
                phase: cols[0].to_string(),
                file: cols[1].to_string(),
                test: cols[2].to_string(),
            }
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum Disposition {
    Ported,
    Excused,
}

#[derive(Debug)]
struct Exception {
    go_file: String,
    go_test: String,
    disposition: Disposition,
    rust_tests: Vec<String>,
    /// `Q-` names and plan-doc paths, comma-separated in the file.
    refs: Vec<String>,
}

/// Parses the exceptions file. Lines starting with `#` are comments. Errors
/// name the offending line.
fn parse_exceptions(source: &str) -> Result<Vec<Exception>, String> {
    let mut lines = source.lines().enumerate();
    match lines.next() {
        Some((_, header)) if header == EXCEPTIONS_HEADER => {}
        other => {
            return Err(format!(
                "expected header {EXCEPTIONS_HEADER:?}, got {other:?}"
            ));
        }
    }
    let mut seen = BTreeSet::new();
    let mut exceptions = Vec::new();
    for (index, line) in lines {
        let n = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let [go_file, go_test, disposition, rust_tests, reference] = cols[..] else {
            return Err(format!(
                "line {n}: want 5 tab-separated columns, got {}",
                cols.len()
            ));
        };
        if go_file.is_empty() || go_test.is_empty() {
            return Err(format!("line {n}: go_file and go_test are required"));
        }
        let rust_tests = list(rust_tests);
        let refs = list(reference);
        if refs.is_empty() {
            return Err(format!("line {n}: {go_test} has no ref"));
        }
        let disposition = match disposition {
            "ported" if rust_tests.is_empty() => {
                return Err(format!(
                    "line {n}: {go_test} is ported but names no Rust tests"
                ));
            }
            "ported" => Disposition::Ported,
            "excused" if !rust_tests.is_empty() => {
                return Err(format!(
                    "line {n}: {go_test} is excused but names Rust tests"
                ));
            }
            "excused" => Disposition::Excused,
            other => {
                return Err(format!(
                    "line {n}: disposition {other:?} is neither ported nor excused"
                ));
            }
        };
        if !seen.insert((go_file.to_string(), go_test.to_string())) {
            return Err(format!("line {n}: duplicate key {go_file} {go_test}"));
        }
        exceptions.push(Exception {
            go_file: go_file.to_string(),
            go_test: go_test.to_string(),
            disposition,
            rust_tests,
            refs,
        });
    }
    Ok(exceptions)
}

/// A comma-separated column, trimmed, empty items dropped.
fn list(column: &str) -> Vec<String> {
    column
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Why `reference` doesn't point at anything, if it doesn't: a `Q-NAME`
/// needs its `{#q-name}` anchor in `open_questions`, and anything else is
/// a path that must exist under the plans directory.
fn unresolved(reference: &str, open_questions: &str, plans: &Path) -> Option<String> {
    if let Some(name) = reference.strip_prefix("Q-") {
        let anchor = format!("{{#q-{}}}", name.to_lowercase());
        return (!open_questions.contains(&anchor))
            .then(|| format!("{reference}: no {anchor} in open-questions.md"));
    }
    (!plans.join(reference).is_file()).then(|| format!("{reference}: no such plan doc"))
}

fn exceptions() -> Vec<Exception> {
    parse_exceptions(&read(EXCEPTIONS)).unwrap_or_else(|e| panic!("{EXCEPTIONS}: {e}"))
}

fn name_matched(rust: &BTreeSet<String>) -> BTreeSet<String> {
    rust.iter().map(|n| normalize_rust(n)).collect()
}

#[test]
fn every_inventory_row_is_ported_or_excused() {
    let matched = name_matched(&rust_tests());
    let excepted: BTreeSet<(String, String)> = exceptions()
        .into_iter()
        .map(|e| (e.go_file, e.go_test))
        .collect();
    let missing: Vec<String> = parse_inventory(&read(INVENTORY))
        .into_iter()
        .filter(|row| !matched.contains(&normalize_go(&row.test)))
        .filter(|row| !excepted.contains(&(row.file.clone(), row.test.clone())))
        .map(|row| format!("{}\t{}\t{}", row.phase, row.file, row.test))
        .collect();
    assert!(
        missing.is_empty(),
        "{} Go tests match no Rust #[test] by name and have no row in {EXCEPTIONS}:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_ported_exception_names_real_tests() {
    let rust = rust_tests();
    let unknown: Vec<String> = exceptions()
        .iter()
        .filter(|e| e.disposition == Disposition::Ported)
        .flat_map(|e| {
            e.rust_tests
                .iter()
                .filter(|t| !rust.contains(*t))
                .map(move |t| format!("{} -> {t}", e.go_test))
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "{EXCEPTIONS} names Rust tests that don't exist:\n{}",
        unknown.join("\n")
    );
}

#[test]
fn no_exception_is_stale() {
    let matched = name_matched(&rust_tests());
    let inventory: BTreeMap<(String, String), InventoryRow> = parse_inventory(&read(INVENTORY))
        .into_iter()
        .map(|row| ((row.file.clone(), row.test.clone()), row))
        .collect();
    let mut stale = Vec::new();
    for e in exceptions() {
        if !inventory.contains_key(&(e.go_file.clone(), e.go_test.clone())) {
            stale.push(format!("{} {}: not in the inventory", e.go_file, e.go_test));
        } else if matched.contains(&normalize_go(&e.go_test)) {
            stale.push(format!(
                "{} {}: already name-matches a Rust test",
                e.go_file, e.go_test
            ));
        }
    }
    assert!(
        stale.is_empty(),
        "stale rows in {EXCEPTIONS}:\n{}",
        stale.join("\n")
    );
}

#[test]
fn every_exception_ref_resolves() {
    let open_questions = read(&format!("{PLANS}/open-questions.md"));
    let plans = repo_root().join(PLANS);
    let broken: Vec<String> = exceptions()
        .iter()
        .flat_map(|e| {
            e.refs
                .iter()
                .filter_map(|r| unresolved(r, &open_questions, &plans))
                .map(move |why| format!("{}: {why}", e.go_test))
        })
        .collect();
    assert!(
        broken.is_empty(),
        "{EXCEPTIONS} has refs a reader can't follow:\n{}",
        broken.join("\n")
    );
}

#[test]
fn a_ref_resolves_to_a_q_anchor_or_an_existing_plan_doc() {
    let plans = repo_root().join(PLANS);
    let oq = "### Q-FOO-BAR — x {#q-foo-bar}\n";
    assert_eq!(unresolved("Q-FOO-BAR", oq, &plans), None);
    assert!(unresolved("Q-FOO", oq, &plans).is_some());
    assert_eq!(unresolved("p9-parity-signoff/index.md", oq, &plans), None);
    assert!(unresolved("p9-parity-signoff/nope.md", oq, &plans).is_some());
    assert!(
        unresolved("p9-parity-signoff", oq, &plans).is_some(),
        "a directory"
    );
}

#[test]
fn go_and_rust_names_normalize_to_the_same_key() {
    assert_eq!(
        normalize_go("TestEditYAMLScratchFileCreated0600"),
        normalize_rust("edit_yaml_scratch_file_created_0600")
    );
    assert_eq!(normalize_go("TestMain"), "main");
    assert_ne!(normalize_go("TestFooBar"), normalize_rust("foo_baz"));
}

#[test]
fn the_scanner_finds_test_fns_through_interleaved_attributes() {
    let source = [
        "#[test]",
        "fn plain() {}",
        "#[test]",
        "#[cfg(unix)]",
        "// a comment",
        "#[cfg_attr(windows, ignore = \"why\")]",
        "",
        "#[ignore]",
        "fn interleaved() {}",
        "#[test] fn one_line() {}",
        "    #[test]",
        "    pub fn indented() {}",
        "fn not_a_test() {}",
        "#[test]",
        "mod m {}",
        "fn not_a_test_either() {}",
    ]
    .join("\n");
    assert_eq!(
        test_fns(&source),
        ["plain", "interleaved", "one_line", "indented"]
    );
}

fn exceptions_with(rows: &[&str]) -> String {
    std::iter::once(EXCEPTIONS_HEADER)
        .chain(rows.iter().copied())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_exceptions_parser_accepts_well_formed_rows() {
    let parsed = parse_exceptions(&exceptions_with(&[
        "# a comment",
        "a_test.go\tTestA\tported\tfoo, bar\tQ-X",
        "",
        "a_test.go\tTestB\texcused\t\tm1.md",
        "b_test.go\tTestA\texcused\t\tQ-Y",
    ]))
    .unwrap();
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].disposition, Disposition::Ported);
    assert_eq!(parsed[0].rust_tests, ["foo", "bar"]);
    assert_eq!(parsed[1].disposition, Disposition::Excused);
    assert!(parsed[1].rust_tests.is_empty());
}

#[test]
fn the_exceptions_parser_rejects_a_missing_ref() {
    let err = parse_exceptions(&exceptions_with(&["a_test.go\tTestA\texcused\t\t"])).unwrap_err();
    assert!(err.contains("no ref"), "{err}");
}

#[test]
fn the_exceptions_parser_rejects_ported_with_no_rust_tests() {
    let err = parse_exceptions(&exceptions_with(&["a_test.go\tTestA\tported\t\tQ-X"])).unwrap_err();
    assert!(err.contains("names no Rust tests"), "{err}");
}

#[test]
fn the_exceptions_parser_rejects_excused_with_rust_tests() {
    let err =
        parse_exceptions(&exceptions_with(&["a_test.go\tTestA\texcused\tfoo\tQ-X"])).unwrap_err();
    assert!(err.contains("excused but names Rust tests"), "{err}");
}

#[test]
fn the_exceptions_parser_rejects_a_duplicate_key() {
    let err = parse_exceptions(&exceptions_with(&[
        "a_test.go\tTestA\texcused\t\tQ-X",
        "a_test.go\tTestA\tported\tfoo\tQ-X",
    ]))
    .unwrap_err();
    assert!(err.contains("duplicate key"), "{err}");
}

#[test]
fn the_exceptions_parser_rejects_a_wrong_header_or_column_count() {
    assert!(parse_exceptions("go_file\tgo_test\n").is_err());
    let err = parse_exceptions(&exceptions_with(&["a_test.go\tTestA\texcused\tQ-X"])).unwrap_err();
    assert!(err.contains("5 tab-separated columns"), "{err}");
}
