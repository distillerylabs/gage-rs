//! gage-rs's own help output, pinned by snapshots in `tests/fixtures/help/`.
//! A help change is then a visible diff in review, never an accident.
//!
//! Surfaces: the root listing, `gage help <path>` for every registered
//! command and parent, the in-session listing, and one usage error per
//! parent. Run with `UPDATE_SNAPSHOTS=1` to rewrite the files; that run
//! fails on purpose, so an update is never silent.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::cli;
use crate::registry::registry;
use crate::version::resolve_build_info;

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/help")
}

/// Every surface as `(snapshot file stem, rendered text)`.
fn surfaces() -> Vec<(String, String)> {
    let reg = registry();
    // Fixed, so a stamped version never reaches a snapshot.
    let info = resolve_build_info(Some("snapshot"), Some("0000000"), "0.0.0");
    let mut out = vec![("root".to_string(), cli::render_root_help(&reg))];
    let mut paths: Vec<&str> = reg
        .one_shot()
        .iter()
        .map(|c| c.name)
        .chain(reg.parents.iter().map(|p| p.name))
        .collect();
    paths.sort();
    for path in paths {
        let topic: Vec<String> = path.split(' ').map(str::to_string).collect();
        let text = cli::help_topic(&reg, &info, &topic).expect("registered path has help");
        out.push((path.replace(' ', "_"), text));
    }
    out.push(("session".into(), cli::render_session_help(&reg)));
    for parent in &reg.parents {
        let args = [parent.name.to_string(), "no-such-subcommand".to_string()];
        let err = cli::parse(&reg, &info, &args, false)
            .err()
            .unwrap_or_else(|| panic!("`{} no-such-subcommand` parsed", parent.name));
        out.push((format!("usage-error_{}", parent.name), err.to_string()));
    }
    out
}

fn snapshot_files() -> BTreeSet<String> {
    std::fs::read_dir(snapshot_dir())
        .expect("tests/fixtures/help exists")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn help_snapshots_match() {
    let dir = snapshot_dir();
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        for stem in snapshot_files() {
            std::fs::remove_file(dir.join(format!("{stem}.txt"))).unwrap();
        }
        for (stem, text) in surfaces() {
            std::fs::write(dir.join(format!("{stem}.txt")), text).unwrap();
        }
        panic!(
            "snapshots rewritten; review `git diff tests/fixtures/help` and rerun without UPDATE_SNAPSHOTS"
        );
    }
    let mut stale = Vec::new();
    for (stem, text) in surfaces() {
        let want = std::fs::read_to_string(dir.join(format!("{stem}.txt"))).unwrap_or_default();
        if want != text {
            stale.push(format!(
                "{stem}.txt differs\n--- snapshot\n{want}\n--- now\n{text}"
            ));
        }
    }
    assert!(
        stale.is_empty(),
        "help changed (UPDATE_SNAPSHOTS=1 to accept):\n{}",
        stale.join("\n")
    );
}

#[test]
fn help_snapshots_cover_every_registered_command() {
    let have = snapshot_files();
    let want: BTreeSet<String> = surfaces().into_iter().map(|(stem, _)| stem).collect();
    assert_eq!(
        have.difference(&want).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "snapshot files with no registered command"
    );
    assert_eq!(
        want.difference(&have).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "registered commands with no snapshot (UPDATE_SNAPSHOTS=1)"
    );
}
