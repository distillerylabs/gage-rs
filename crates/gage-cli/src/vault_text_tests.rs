//! Text from a vault reaches the terminal as text (ADR-0069), through the
//! real CLI, in-process. The prompts are tested beside the prompter
//! (`prompter.rs`), which is what draws them.

use gage::prompter::FakePrompter;

use crate::entry_tests::{env_with_vault, unlocks};
use crate::exitcode::ExitCode;
use crate::lifecycle_tests::CliResult;
use crate::trustcache_cli_tests::tamper_recipients_file;

const VAULT: &str = "personal";

/// Would erase the line it lands on, and add one of its own.
const HOSTILE_TITLE: &str = "mail\x1b[2K\rforged\nsecond line";
const HOSTILE_KEY: &str = "age1evil\x1b[2Kkey";
const SHOWN_KEY: &str = r"age1evil\u{1b}[2Kkey";

fn assert_only_text(r: &CliResult) {
    for out in [&r.stdout, &r.stderr] {
        assert!(!out.chars().any(|c| c.is_control() && c != '\n'), "{out:?}");
    }
}

#[test]
fn ls_shows_a_hostile_title_as_one_row_of_text() {
    let env = env_with_vault();
    env.insert_ok("v", &[HOSTILE_TITLE]);

    let r = env.cmd_ok(&["ls"]);

    assert_only_text(&r);
    assert_eq!(r.stdout.lines().count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains(r"mail\u{1b}[2K\rforged\nsecond line"));
}

#[test]
fn search_shows_a_hostile_title_as_one_row_of_text() {
    let env = env_with_vault();
    env.insert_ok("v", &[HOSTILE_TITLE]);

    let r = env.cmd_ok(&["search", "forged"]);

    assert_only_text(&r);
    assert_eq!(r.stdout.lines().count(), 1, "{}", r.stdout);
}

#[test]
fn an_ambiguous_match_lists_hostile_titles_one_per_line() {
    let env = env_with_vault();
    env.insert_ok("v", &["dup\x1b[1A\none"]);
    env.insert_ok("v", &["dup\x1b[1A\ntwo"]);

    let r = env.run_with(&["show", "dup"], &unlocks());

    assert_eq!(r.code, ExitCode::Ambiguous, "{}", r.stderr);
    assert_only_text(&r);
    assert_eq!(r.stderr.lines().count(), 3, "{}", r.stderr);
}

// Secret output exists to be piped: what went in comes out.
#[test]
fn show_prints_a_value_byte_for_byte() {
    let env = env_with_vault();
    env.insert_ok("a\x1b[0mb", &["Plain"]);

    assert_eq!(env.cmd_ok(&["show", "Plain"]).stdout, "a\x1b[0mb\n");
}

// `cat` is YAML, whose own escaping already keeps control characters out.
#[test]
fn cat_keeps_its_yaml_escaping_untouched() {
    let env = env_with_vault();
    env.insert_ok("a\x1b[0mb", &["Plain"]);

    let r = env.cmd_ok(&["cat", "Plain"]);

    assert_only_text(&r);
    assert!(r.stdout.contains(r"a\x1B[0mb"), "{}", r.stdout);
}

#[test]
fn recipient_verify_shows_a_hostile_line_as_text() {
    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), HOSTILE_KEY);

    let r = env.run_with(&["recipient", "verify"], &FakePrompter::new());

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert_only_text(&r);
    assert!(r.stderr.contains(SHOWN_KEY), "{}", r.stderr);
}

// A line that isn't a key is a difference like any other, so the exit
// from one still works.
#[test]
fn repair_drops_a_hostile_line() {
    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), HOSTILE_KEY);

    let p = FakePrompter::new().with_confirm(true);
    let r = env.run_with(&["recipient", "verify", "--repair"], &p);

    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_only_text(&r);
    assert!(r.stdout.contains(SHOWN_KEY), "{}", r.stdout);
    let none = FakePrompter::new();
    assert_eq!(
        env.run_with(&["recipient", "verify"], &none).code,
        ExitCode::Success
    );
}

// The error quotes what it refused, and that is vault text too.
#[test]
fn a_config_pubkey_that_is_no_key_is_refused_as_text() {
    let env = env_with_vault();
    let path = env.vault_dir().join(".gage/config.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let key = env.entry(VAULT).pubkey.unwrap();
    assert!(text.contains(&key));
    std::fs::write(&path, text.replace(&key, r"age1evil\u001b[2Kkey")).unwrap();

    let r = env.run_with(&["recipient", "list"], &FakePrompter::new());

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert_only_text(&r);
    assert!(r.stderr.contains("is not an age recipient"), "{}", r.stderr);
}
