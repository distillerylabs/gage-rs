//! `crates/gage` must not depend, directly or transitively, on any
//! frontend concern: argument parsing, terminal input, line editing,
//! clipboard, or QR rendering. A future GUI/TUI is a second in-process
//! frontend over this library, so a frontend crate leaking in through a
//! dependency would quietly couple the two. The lint in `clippy.toml`
//! guards the code; this guards the dependency graph.
//!
//! It is a test rather than a CI-only script so it runs in `make test` on
//! every platform. Dev-dependencies are exempt (tests may use anything);
//! build and normal dependencies are not.

use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Command;

use serde_json::Value;

/// A trailing `*` matches any crate name with that prefix (`clap*` covers
/// `clap`, `clap_builder`, `clap_derive`, ...).
const DENYLIST: &[&str] = &[
    "clap*",
    "rpassword",
    "crossterm",
    "termion",
    "rustyline",
    "reedline",
    "arboard",
    "qrcode",
    "dialoguer",
    "inquire",
];

fn is_denied(name: &str) -> bool {
    DENYLIST
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *pattern,
        })
}

/// Every denylisted crate reachable from `root_name` through normal or
/// build dependencies, each as the shortest path that pulls it in
/// (`["gage", "foo", "clap"]`). Panics if `root_name` isn't in the metadata,
/// so a rename can't turn the check into a vacuous pass.
fn offenders(metadata: &Value, root_name: &str) -> Vec<Vec<String>> {
    let packages = metadata["packages"].as_array().expect("packages array");
    let names: HashMap<&str, &str> = packages
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap()))
        .collect();
    let root = packages
        .iter()
        .find(|p| p["name"] == root_name && p["source"].is_null())
        .unwrap_or_else(|| panic!("workspace package {root_name:?} not found in metadata"))["id"]
        .as_str()
        .unwrap();

    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .expect("resolve nodes");
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in nodes {
        let from = node["id"].as_str().unwrap();
        for dep in node["deps"].as_array().unwrap() {
            // dev-only edges don't ship; null (normal) and "build" do.
            let ships = dep["dep_kinds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|k| k["kind"] != "dev");
            if ships {
                edges
                    .entry(from)
                    .or_default()
                    .push(dep["pkg"].as_str().unwrap());
            }
        }
    }

    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);
    let mut found = Vec::new();
    while let Some(id) = queue.pop_front() {
        for &next in edges.get(id).into_iter().flatten() {
            if !seen.insert(next) {
                continue;
            }
            parent.insert(next, id);
            queue.push_back(next);
            if is_denied(names[next]) {
                let mut path = vec![names[next].to_string()];
                let mut cursor = next;
                while let Some(&up) = parent.get(cursor) {
                    path.push(names[up].to_string());
                    cursor = up;
                }
                path.reverse();
                found.push(path);
            }
        }
    }
    found
}

fn fixture(deps_of_gage: &[(&str, &str)], deps_of_foo: &[(&str, &str)]) -> Value {
    let dep = |id: &str, kind: &str| {
        let kind: Value = if kind == "normal" {
            Value::Null
        } else {
            kind.into()
        };
        serde_json::json!({ "pkg": id, "dep_kinds": [{ "kind": kind, "target": null }] })
    };
    let deps = |list: &[(&str, &str)]| -> Vec<Value> {
        list.iter().map(|(id, kind)| dep(id, kind)).collect()
    };
    serde_json::json!({
        "packages": [
            { "id": "gage", "name": "gage", "source": null },
            { "id": "foo", "name": "foo", "source": "registry+x" },
            { "id": "clap", "name": "clap", "source": "registry+x" },
            { "id": "serde", "name": "serde", "source": "registry+x" },
        ],
        "resolve": { "nodes": [
            { "id": "gage", "deps": deps(deps_of_gage) },
            { "id": "foo", "deps": deps(deps_of_foo) },
            { "id": "clap", "deps": [] },
            { "id": "serde", "deps": [] },
        ] },
    })
}

#[test]
fn the_real_library_depends_on_no_frontend_crate() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    // No --offline: `cargo metadata` needs every package in the lockfile on
    // disk, which a bare `cargo test` hasn't fetched. `make test` fetches
    // first and sets CARGO_NET_OFFLINE=true, which this command inherits, so
    // the suite still can't reach the network where that matters.
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");

    assert_eq!(offenders(&metadata, "gage"), Vec::<Vec<String>>::new());
}

/// Host SDKs and OAuth/device-flow crates: how host-specific code paths
/// would come back (Go's Q-OAUTH-APP). gage's remote auth is a token the
/// user issued, for any host.
const HOST_SPECIFIC: &[&str] = &[
    "oauth2",
    "yup-oauth2",
    "octocrab",
    "github*",
    "gitlab*",
    "hubcaps",
    "gix-credentials",
];

fn is_host_specific(name: &str) -> bool {
    HOST_SPECIFIC
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *pattern,
        })
}

// Go: TestNoHostSpecificDependencies: no host SDK or OAuth crate anywhere
// in the workspace's graph, and no OAuth client id in its sources.
#[test]
fn no_host_specific_dependencies() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo metadata");
    assert!(output.status.success());
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("metadata is JSON");
    let offenders: Vec<&str> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["name"].as_str())
        .filter(|name| is_host_specific(name))
        .collect();
    assert_eq!(offenders, Vec::<&str>::new());
    assert!(is_host_specific("octocrab") && is_host_specific("github-rs"));

    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut stack = vec![crates];
    let this_file = std::path::Path::new(file!()).file_name().unwrap();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs" || e == "toml")
                && path.file_name() != Some(this_file)
            {
                let text = std::fs::read_to_string(&path).unwrap();
                for needle in ["client_id", "client-id", "device_code"] {
                    assert!(
                        !text.contains(needle),
                        "{} contains {needle}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
fn a_transitive_frontend_dependency_is_reported_with_the_path_that_pulls_it_in() {
    let metadata = fixture(
        &[("foo", "normal")],
        &[("clap", "normal"), ("serde", "normal")],
    );

    assert_eq!(
        offenders(&metadata, "gage"),
        vec![vec![
            "gage".to_string(),
            "foo".to_string(),
            "clap".to_string()
        ]]
    );
}

#[test]
fn a_build_dependency_counts() {
    let metadata = fixture(&[("clap", "build")], &[]);

    assert_eq!(
        offenders(&metadata, "gage"),
        vec![vec!["gage".to_string(), "clap".to_string()]]
    );
}

#[test]
fn a_frontend_crate_reachable_only_through_dev_dependencies_is_not_reported() {
    let metadata = fixture(&[("clap", "dev"), ("foo", "dev")], &[("clap", "normal")]);

    assert_eq!(offenders(&metadata, "gage"), Vec::<Vec<String>>::new());
}

#[test]
#[should_panic(expected = "not found in metadata")]
fn a_missing_root_package_is_a_failure_not_a_pass() {
    offenders(&fixture(&[], &[]), "no-such-package");
}

#[test]
fn the_denylist_matches_prefixes_and_exact_names() {
    for denied in [
        "clap",
        "clap_builder",
        "clap_derive",
        "crossterm",
        "rustyline",
        "qrcode",
    ] {
        assert!(is_denied(denied), "{denied} should be denied");
    }
    for allowed in ["serde", "serde_json", "age", "toml", "uuid"] {
        assert!(!is_denied(allowed), "{allowed} should be allowed");
    }
}
