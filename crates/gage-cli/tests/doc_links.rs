//! Every relative Markdown link outside `doc/archive/` resolves, and so does
//! every `#anchor` it names. `std::fs` only: no link-checking dependency.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Directories whose Markdown is not checked: build output, VCS, vendored or
/// generated fixtures, and the frozen archive.
fn skipped(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s == "target"
        || s == "bin"
        || s == ".git"
        || s == "doc/archive"
        || s.starts_with("tests/fixtures")
        || s.starts_with("node_modules")
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap();
        if skipped(rel) {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// GitHub's heading slug: lowercase, keep alphanumerics, `-` and `_`, spaces
/// become `-`.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Anchors a Markdown file defines: heading slugs (with GitHub's `-N` suffix
/// for repeats), explicit `{#id}` heading ids and `<a id=".."|name="..">`.
fn anchors(text: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if line.starts_with('#') {
            let mut title = line.trim_start_matches('#').trim().to_string();
            if let Some(i) = title.rfind("{#")
                && let Some(id) = title[i + 2..].strip_suffix('}')
            {
                set.insert(id.to_string());
                title.truncate(i);
            }
            let base = slug(&title.replace('`', ""));
            let n = seen.entry(base.clone()).or_insert(0);
            set.insert(if *n == 0 { base } else { format!("{base}-{n}") });
            *n += 1;
        }
        for marker in ["id=\"", "name=\""] {
            let mut rest = line;
            while let Some(i) = rest.find(marker) {
                rest = &rest[i + marker.len()..];
                if let Some(j) = rest.find('"') {
                    set.insert(rest[..j].to_string());
                }
            }
        }
    }
    set
}

/// The `(target)` of every inline `[text](target)` link outside code fences
/// and inline code spans.
fn links(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (n, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut plain = String::new();
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code {
                plain.push(c);
            }
        }
        let mut rest = plain.as_str();
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            if let Some(j) = rest.find(')') {
                let target = rest[..j].split_whitespace().next().unwrap_or("");
                out.push((n + 1, target.to_string()));
                rest = &rest[j..];
            }
        }
    }
    out
}

#[test]
fn doc_links_resolve() {
    let root = repo_root();
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    assert!(!files.is_empty(), "no Markdown files found under {root:?}");

    let mut broken = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        for (line, target) in links(&text) {
            if target.is_empty()
                || target.contains("://")
                || target.starts_with("mailto:")
                || target.starts_with('<')
            {
                continue;
            }
            let (path_part, anchor) = match target.split_once('#') {
                Some((p, a)) => (p, Some(a)),
                None => (target.as_str(), None),
            };
            let dest = if path_part.is_empty() {
                file.clone()
            } else {
                file.parent().unwrap().join(path_part)
            };
            let shown = file.strip_prefix(&root).unwrap().display();
            if !dest.exists() {
                broken.push(format!("{shown}:{line}: {target}: no such path"));
                continue;
            }
            if let Some(a) = anchor.filter(|a| !a.is_empty())
                && dest.extension().is_some_and(|e| e == "md")
            {
                let found = anchors(&fs::read_to_string(&dest).unwrap());
                if !found.contains(a) {
                    broken.push(format!("{shown}:{line}: {target}: no such anchor"));
                }
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} broken doc link(s):\n{}",
        broken.len(),
        broken.join("\n")
    );
}
