//! The source carries no provenance from the port: no reference to the
//! implementation it began as, its test names, its question register or its
//! milestone ids. A comment says why; a decision that binds code cites its
//! ADR in `doc/decisions/`.
//!
//! `std::fs` only, like `doc_links.rs`. A hit is reported as `path:line: text`.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the scan looks, relative to the repo root. A directory is walked for
/// `.rs`, `.toml` and `.sh` files; a file is scanned as it is.
const ROOTS: &[&str] = &[
    "crates/gage",
    "crates/gage-cli",
    "scripts/qa",
    "Cargo.toml",
    "deny.toml",
    "rust-toolchain.toml",
];

/// `(path, substring, reason)`: a line under `path` containing `substring` is
/// not reported. Empty unless a real false positive survives.
const ALLOWED: &[(&str, &str, &str)] = &[];

/// The guard names every anchor it looks for, in its detector and in the
/// lines that prove the detector fires, so it cannot pass its own scan.
const THIS_FILE: &str = "crates/gage-cli/tests/port_anchors.rs";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn rel_str(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "rs" || e == "toml" || e == "sh")
        {
            out.push(path);
        }
    }
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in ROOTS {
        let p = root.join(r);
        if p.is_dir() {
            collect(&p, &mut out);
        } else {
            assert!(p.is_file(), "scan root {r} does not exist");
            out.push(p);
        }
    }
    out.sort();
    out
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `\w+` runs of `line`, with the byte offset each starts at.
fn words(line: &str) -> Vec<(usize, &str)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if is_word(b[i]) {
            let s = i;
            while i < b.len() && is_word(b[i]) {
                i += 1;
            }
            out.push((s, &line[s..i]));
        } else {
            i += 1;
        }
    }
    out
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `<prefix><digits>[a-z]?` as a whole word.
fn numbered(word: &str, prefix: &str, max_digits: usize) -> bool {
    let Some(rest) = word.strip_prefix(prefix) else {
        return false;
    };
    let digits = rest.trim_end_matches(|c: char| c.is_ascii_lowercase());
    let letters = rest.len() - digits.len();
    all_digits(digits) && digits.len() <= max_digits && letters <= 1
}

/// `Test` + an uppercase letter, lowercase letters, an uppercase letter: the
/// shape of a Go test function name.
fn go_test_name(line: &str) -> bool {
    let b = line.as_bytes();
    let mut from = 0;
    while let Some(at) = line[from..].find("Test") {
        let mut i = from + at + 4;
        from += at + 4;
        if i < b.len() && b[i].is_ascii_uppercase() {
            i += 1;
            let lower = b[i..].iter().take_while(|c| c.is_ascii_lowercase()).count();
            i += lower;
            if lower > 0 && i < b.len() && b[i].is_ascii_uppercase() {
                return true;
            }
        }
    }
    false
}

/// What the line carries, if anything: the first anchor found.
fn anchor(line: &str) -> Option<&'static str> {
    if line.contains(concat!("// ", "Go:")) {
        return Some("Go comment");
    }
    if go_test_name(line) {
        return Some("Go test name");
    }
    let b = line.as_bytes();
    let mut from = 0;
    while let Some(at) = line[from..].find(".go") {
        let end = from + at + 3;
        from = end;
        if end >= b.len() || !is_word(b[end]) {
            return Some("Go file name");
        }
    }
    for (at, w) in words(line) {
        let after = &line[at + w.len()..];
        if w == "Go" {
            return Some("the word Go");
        }
        if w == "Q"
            && after
                .strip_prefix('-')
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_uppercase()))
        {
            return Some("question id");
        }
        if numbered(w, "M", 4)
            || numbered(w, "RK", 4)
            || numbered(w, "P", 4)
            || numbered(w, "S", 4)
            || numbered(w, "E", 1)
        {
            return Some("milestone id");
        }
        if numbered(w, "A", 2) && !w[1..].bytes().any(|c| c.is_ascii_lowercase()) {
            return Some("register label");
        }
        if w == "D" && after.starts_with('-') {
            let run = after[1..]
                .bytes()
                .take_while(|c| c.is_ascii_uppercase())
                .count();
            if run > 0 && after[1 + run..].starts_with('-') {
                return Some("design label");
            }
        }
    }
    None
}

fn allowed(rel: &str, line: &str) -> bool {
    ALLOWED
        .iter()
        .any(|(path, sub, _)| *path == rel && line.contains(sub))
}

#[test]
fn no_port_anchors() {
    let root = repo_root();
    let mut hits = Vec::new();
    for path in files(&root) {
        let rel = rel_str(&root, &path);
        if rel == THIS_FILE {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read source file");
        for (i, line) in text.lines().enumerate() {
            if let Some(kind) = anchor(line)
                && !allowed(&rel, line)
            {
                hits.push(format!("{rel}:{}: {kind}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} port anchor(s) in the source; say the reason instead, or cite its ADR:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn allowlisted_entries_still_match() {
    let root = repo_root();
    for (path, sub, reason) in ALLOWED {
        assert!(
            !reason.is_empty(),
            "allowlist entry {path}: {sub} needs a reason"
        );
        let text = fs::read_to_string(root.join(path))
            .unwrap_or_else(|e| panic!("allowlisted file {path}: {e}"));
        assert!(
            text.lines().any(|l| l.contains(sub) && anchor(l).is_some()),
            "allowlist entry {path}: {sub:?} no longer matches an anchor; remove it"
        );
    }
}

#[test]
fn anchor_detector_recognises_each_pattern() {
    let go_comment = format!("x(); // {}: TestFoo", "Go");
    for line in [
        go_comment.as_str(),
        "ports the Go code",
        "see foo_test.go",
        "fn TestRoundTripOk",
        "Q-GIT-CRATE decided",
        "landed in M10",
        "RK4 and P5 and S1",
        "per A19",
        "D-ENROLL-TTL",
    ] {
        assert!(anchor(line).is_some(), "should flag: {line}");
    }
    for line in [
        "let go = 1;",
        "Google Gone",
        "a Testing note, TestSuite",
        "SHA256, M and S alone",
        "Q alone; A; D-day",
        "A3x",
    ] {
        assert!(anchor(line).is_none(), "should not flag: {line}");
    }
}
