//! `crates/gage` speaks for no frontend (ADR-0054): no error or warning
//! names gage (`gage: …`), and no message tells someone to run a `gage`
//! command. A frontend adds its own name and renders a typed `Remedy`.
//!
//! Commit messages are the exception. They are on-disk format
//! (`doc/format/v2.md`), not UI, so a `"gage: ` literal is allowed only as
//! the start of one of the commit subjects below.
//!
//! Std-only and line-based, like `no_frontend_deps.rs`: it reads the
//! library's source, skipping comments, `_tests.rs` files and inline test
//! modules.

use std::path::{Path, PathBuf};

/// The commit subjects gage writes, by their fixed opening words.
const COMMIT_SUBJECTS: &[&str] = &[
    "gage: initialize vault",
    "gage: merge origin",
    "gage: enrollment request",
    "gage: recipient add ",
    "gage: recipient remove ",
    "gage: recipient approve ",
    "gage: recipient repair",
    "gage: recovery rotate",
    "gage: recovery enroll ",
];

/// What's wrong with one line of library code, if anything.
fn offense(line: &str) -> Option<&'static str> {
    let code = line.trim_start();
    if code.starts_with("//") {
        return None;
    }
    let mut rest = code;
    while let Some(at) = rest.find("\"gage: ") {
        let literal = &rest[at + 1..];
        if !COMMIT_SUBJECTS.iter().any(|s| literal.starts_with(s)) {
            return Some("a message names gage; the frontend adds its name");
        }
        rest = &rest[at + 1..];
    }
    if code.contains('"') && code.contains("`gage ") {
        return Some("a message names a gage command; return a Remedy");
    }
    None
}

fn library_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            library_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && !path.to_string_lossy().ends_with("_tests.rs")
        {
            out.push(path);
        }
    }
}

/// Every offending line, as `path:line: why`. A file is read up to its
/// inline test module, which is test code like a `_tests.rs` file.
fn offenses(src: &Path) -> Vec<String> {
    let mut files = Vec::new();
    library_files(src, &mut files);
    assert!(
        files.len() > 20,
        "found only {} files under {src:?}",
        files.len()
    );
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let next = lines.get(i + 1).map_or("", |l| l.trim());
            if line.trim() == "#[cfg(test)]" && next.starts_with("mod ") && next.ends_with('{') {
                break;
            }
            if let Some(why) = offense(line) {
                found.push(format!("{}:{}: {why}", file.display(), i + 1));
            }
        }
    }
    found
}

#[test]
fn the_library_names_no_frontend() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = offenses(&src);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

// The detector fires on each shape it exists for, and spares commit
// messages and comments, so a pass above means something.
#[test]
fn the_detector_fires() {
    for bad in [
        r#"    #[error("gage: no such vault {0:?}")]"#,
        r#"        "gage: {vault:?} has no remote""#,
        r#"        prompter.warn(&Warning::from("gage: careful"));"#,
        r#"    "origin has diverged; run `gage sync`","#,
    ] {
        assert!(offense(bad).is_some(), "missed: {bad}");
    }
    for fine in [
        r#"pub const INIT_COMMIT_MESSAGE: &str = "gage: initialize vault";"#,
        r#"    let message = format!("gage: recipient add {device} (reencrypt)");"#,
        r#"    /// run `gage sync` to resolve it"#,
        r#"    #[error("no such vault {0:?}")]"#,
    ] {
        assert_eq!(offense(fine), None, "flagged: {fine}");
    }
}
