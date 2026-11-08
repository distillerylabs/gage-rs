//! What a person sees when an entry fails authentication (ADR-0065),
//! through the real CLI, in-process. One bad entry fails the command, as a
//! corrupt one does: a listing never looks complete when it isn't.

use gage::Vault;
use gage::crypto::crypt::{self, Recipient};
use gage::sync::gitrepo;
use gage::trust::recipient;
use uuid::Uuid;

use crate::entry_tests::env_with_vault;
use crate::exitcode::ExitCode;
use crate::lifecycle_tests::CliResult;

const SECRET: &str = "forged-secret-value";

/// Commits what anyone with write access to the repository can make: a
/// well-formed payload sealed to the public recipients, with no `auth` line.
fn commit_forgery(vault: &Vault) -> Uuid {
    let to: Vec<Recipient> = recipient::load(&vault.path.join(".age-recipients"))
        .unwrap()
        .iter()
        .map(|key| Recipient::parse(key).unwrap())
        .collect();
    let payload = format!("title: Forged\nvalue: {SECRET}\n");
    let id = Uuid::new_v4();
    std::fs::write(
        vault.path.join(format!("entries/{id}.age")),
        crypt::encrypt(payload.as_bytes(), &to).unwrap(),
    )
    .unwrap();
    gitrepo::commit_all(&vault.path, &id.to_string()).unwrap();
    id
}

fn assert_refused(r: &CliResult, id: Uuid) {
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(
        r.stderr
            .contains(&format!("gage: entry {id} failed authentication")),
        "{}",
        r.stderr
    );
    assert!(r.stdout.is_empty(), "no partial output:\n{}", r.stdout);
    assert!(!r.stderr.contains(SECRET), "{}", r.stderr);
}

#[test]
fn every_read_of_a_forged_entry_exits_conflict_naming_it() {
    let env = env_with_vault();
    env.insert_ok("v", &["Genuine"]);
    let id = commit_forgery(&env.vault("personal"));
    let id_text = id.to_string();

    for args in [
        vec!["cat", id_text.as_str()],
        vec!["show", id_text.as_str()],
        vec!["ls"],
        vec!["search", "Genuine"],
    ] {
        let r = env.cmd(&args);
        assert_refused(&r, id);
        assert!(r.stderr.contains("no auth tag"), "{args:?}: {}", r.stderr);
    }
}

// Takes no key at all: anyone with write access can copy a file.
#[test]
fn a_genuine_entry_copied_over_another_is_refused_as_a_mismatch() {
    let env = env_with_vault();
    let source = env.insert_ok("source-value", &["Source"]);
    let victim = env.insert_ok("victim-value", &["Victim"]);
    let vault = env.vault("personal");
    std::fs::copy(
        vault.path.join(format!("entries/{source}.age")),
        vault.path.join(format!("entries/{victim}.age")),
    )
    .unwrap();

    let r = env.cmd(&["cat", &victim.to_string()]);

    assert_refused(&r, victim);
    assert!(r.stderr.contains("does not match"), "{}", r.stderr);
    assert!(!r.stderr.contains("source-value"), "{}", r.stderr);
}

#[test]
fn one_forged_entry_fails_the_listing_and_no_row_is_printed() {
    let env = env_with_vault();
    for title in ["Alpha", "Beta", "Gamma"] {
        env.insert_ok("v", &[title]);
    }
    let vault = env.vault("personal");
    let id = commit_forgery(&vault);

    assert_refused(&env.cmd(&["ls"]), id);
    // Addressing a sound entry looks through every title, so it fails too.
    assert_refused(&env.cmd(&["cat", "Alpha"]), id);

    // doc/cli/entries.md's way out: gage can't address the entry either,
    // and removing the file with git brings the vault back.
    let file = vault.path.join(format!("entries/{id}.age"));
    assert_refused(&env.cmd(&["rm", &id.to_string()]), id);
    assert!(file.exists());
    std::fs::remove_file(&file).unwrap();
    gitrepo::commit_all(&vault.path, &id.to_string()).unwrap();

    let listed = env.cmd_ok(&["ls"]);
    for title in ["Alpha", "Beta", "Gamma"] {
        assert!(listed.stdout.contains(title), "{}", listed.stdout);
    }
}

// The session's index is built by the same pass, and a script stops at
// the line that hit it, with that line's own code.
#[test]
fn a_script_stops_at_the_line_whose_index_build_hit_a_forged_entry() {
    let env = env_with_vault();
    env.insert_ok("v", &["Genuine"]);
    let id = commit_forgery(&env.vault("personal"));

    let r = env.run_script(&["ls", "cat Genuine"]);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(
        r.stderr
            .contains(&format!(":1: ls: entry {id} failed authentication")),
        "{}",
        r.stderr
    );
    assert!(r.stdout.is_empty(), "{}", r.stdout);
    assert!(!r.stderr.contains(SECRET), "{}", r.stderr);
}
