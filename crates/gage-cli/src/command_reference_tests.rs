//! `doc/cli/command-reference.md` is hand-written, so this
//! keeps it honest: every registered command path and every alias appears
//! in it, as an inline code span in a heading or table row. A command added
//! to the registry without a line in the page fails here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::registry::registry;

fn page() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../doc/cli/command-reference.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The inline code spans on the page's heading and table lines.
fn spans(page: &str) -> Vec<String> {
    page.lines()
        .filter(|l| l.starts_with('|') || l.starts_with('#'))
        .flat_map(|l| {
            l.split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A span names `name` when it is `name` alone or `name` followed by its
/// arguments or flags, so `vault` does not stand in for `vault list`.
fn names(span: &str, name: &str) -> bool {
    span == name
        || span
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(|rest| rest.starts_with(['<', '[', '-']))
}

#[test]
fn command_reference_names_every_registered_command() {
    let spans = spans(&page());
    let reg = registry();
    let mut missing = BTreeSet::new();
    for info in &reg.commands {
        for name in std::iter::once(&info.name).chain(info.aliases) {
            if !spans.iter().any(|s| names(s, name)) {
                missing.insert(*name);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "doc/cli/command-reference.md does not name: {missing:?}"
    );
    // The page was actually read: a parse that quietly found no spans would
    // have reported every command missing, but a page of prose would not.
    assert!(spans.len() > reg.commands.len(), "{} spans", spans.len());
}

#[test]
fn names_matches_a_command_or_its_arguments_not_a_prefix() {
    assert!(names("sync", "sync"));
    assert!(names("sync [--use NAME]", "sync"));
    assert!(names("vault list", "vault list"));
    assert!(names(
        "recipient add <pubkey> --device NAME",
        "recipient add"
    ));
    assert!(!names("vault list", "vault"));
    assert!(!names("synchronize", "sync"));
    assert!(!names("vault info [<name>]", "vault list"));
}
