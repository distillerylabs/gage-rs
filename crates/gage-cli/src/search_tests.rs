//! `search`/`grep`, `reindex`, and the session's metadata index, through
//! the real CLI in-process: one-shot, and in a session driven by a typed
//! script.

use gage::exitcode::Code;

use crate::editor::fake::{Action, FakeEditor, Set};
use crate::entry_tests::env_with_vault;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::repl_tests::passphrase_prompts;

/// The lines a session printed that aren't prompts, passphrase requests
/// or blank: its command output, in order.
fn session_rows(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(|l| l.rsplit("gage> ").next().unwrap())
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Everything after the last line containing `marker`.
fn after_last<'a>(text: &'a str, marker: &str) -> &'a str {
    let at = text
        .rfind(marker)
        .unwrap_or_else(|| panic!("{marker:?} never appeared in:\n{text}"));
    &text[at..]
}

fn search_fixture(env: &Env) {
    env.add("personal", "AWS root account", "x");
    env.insert_ok(
        "y",
        &["Cloud creds", "--description", "shared aws credentials"],
    );
    env.add("personal", "Unrelated", "the aws secret key is here");
    env.add("personal", "Nothing", "irrelevant");
}

// A title, a
// description and a body match, printed as `title  shortid` with no
// matched text; `grep` is the same command.
#[test]
fn search_one_shot_matches_title_description_and_body() {
    let env = env_with_vault();
    search_fixture(&env);

    for cmd in ["search", "grep"] {
        let r = env.cmd_ok(&[cmd, "aws"]);
        let titles: Vec<&str> = r
            .stdout
            .lines()
            .map(|l| l.rsplit_once("  ").unwrap().0)
            .collect();
        assert_eq!(
            titles,
            ["AWS root account", "Cloud creds", "Unrelated"],
            "{cmd}"
        );
        for line in r.stdout.lines() {
            let short = line.rsplit_once("  ").unwrap().1;
            assert_eq!(short.len(), 8, "{line}");
        }
        assert!(
            !r.stdout.contains("secret key"),
            "{cmd} leaked:\n{}",
            r.stdout
        );
        assert!(
            !r.stdout.contains("credentials"),
            "{cmd} leaked:\n{}",
            r.stdout
        );
    }
}

// The search half (the
// ls half is ls_empty_vault_no_output). No rows means no output at
// all, so `gage search x | wc -l` is 0.
#[test]
fn search_printing_nothing_when_there_is_nothing() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["search", "nothing-matches-this"]);
    assert_eq!(r.stdout, "");
    env.add("personal", "Something", "v");
    let r = env.cmd_ok(&["search", "nothing-matches-this"]);
    assert_eq!(r.stdout, "");
}

// One-shot never caches, so
// reindex says so, after checking the vault resolves, without unlocking.
#[test]
fn reindex_one_shot_has_nothing_to_rebuild() {
    let env = env_with_vault();
    // No passphrase scripted: an unlock would fail the test.
    let r = env.run_with(&["reindex"], &gage::prompter::FakePrompter::new());
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("nothing to reindex"), "{}", r.stdout);

    let r = env.run_with(
        &["reindex", "--use", "no-such-vault"],
        &gage::prompter::FakePrompter::new(),
    );
    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
}

// Ls and search follow
// the vault as it changes, through the real REPL, on one passphrase.
#[test]
fn session_ls_search_reindex_see_insert_rename_and_rm() {
    let env = env_with_vault();
    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "ls",
        "insert AWS",
        "correcthorsebatterystaple",
        "ls",
        "search AWS",
        "reindex",
        "ls",
        "rename AWS Amazon",
        "ls",
        "search Amazon",
        "rm Amazon",
        "ls",
        "exit",
    ]);
    assert_eq!(passphrase_prompts(&r), 1, "{}", r.stderr);
    let out = &r.stdout;

    let inserted = after_last(out, "gage: inserted");
    let renamed_at = inserted.find("gage: renamed").unwrap();
    let before_rename = &inserted[..renamed_at];
    assert_eq!(
        before_rename.matches("AWS  ").count(),
        3,
        "ls, search and ls after reindex each show AWS:\n{before_rename}"
    );
    assert!(before_rename.contains("gage: index rebuilt"), "{out}");

    let renamed = after_last(out, "gage: renamed");
    let removed_at = renamed.find("gage: removed").unwrap();
    assert_eq!(
        renamed[..removed_at].matches("Amazon  ").count(),
        2,
        "ls and search see the new title:\n{renamed}"
    );
    assert!(!renamed[..removed_at].contains("AWS"), "{renamed}");

    let removed = after_last(out, "gage: removed");
    assert!(!removed.contains("Amazon"), "ls after rm:\n{removed}");
    assert!(env.entries("personal").is_empty());
}

// A session that already
// listed sees an $EDITOR title change in its very next ls.
#[test]
fn session_edit_updates_index_without_reindex() {
    let env = env_with_vault();
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");
    let fake = FakeEditor::new(Action::Set(&[Set::Title("Proton Mail")]));
    env.set_editor(&fake);

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "ls",
        "edit ProtonMail",
        "ls",
        "exit",
    ]);
    let tail = after_last(&r.stdout, "gage: updated");
    assert!(tail.contains("Proton Mail  "), "{tail}");
    assert!(!tail.contains("ProtonMail"), "{tail}");
}

// generate updates the index too (insert's sibling), and a vault touched
// with --use gets its own.
#[test]
fn session_generate_and_use_flag_keep_their_indexes_current() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "ls",
        "generate Fresh",
        "ls",
        "ls --use work",
        PASSPHRASE,
        "insert --use work Jira",
        "jira-secret",
        "ls --use work",
        "exit",
    ]);
    let rows = session_rows(&r.stdout);
    let generated = rows
        .iter()
        .position(|l| l.contains("gage: generated"))
        .unwrap();
    assert!(rows[generated + 1].starts_with("Fresh  "), "{rows:?}");
    let last = rows.iter().rposition(|l| l.starts_with("Jira  ")).unwrap();
    assert!(
        rows[last - 1].contains("gage: inserted \"Jira\""),
        "{rows:?}"
    );
}

// Two implementations
// behind `ls` (a fresh decrypt one-shot, the index in a session), one
// renderer: the rows match, alignment and the id tie-break included.
#[test]
fn ls_renders_identically_in_both_modes() {
    let env = env_with_vault();
    for title in ["AWS root account", "a", "Café"] {
        env.add("personal", title, &format!("v-{title}"));
    }
    env.insert_ok("v-a-2", &["-f", "a"]);

    let one_shot = env.cmd_ok(&["ls"]).stdout;
    let session = env.session_ok(&["use personal", PASSPHRASE, "ls", "exit"]);
    let want: Vec<&str> = one_shot.lines().collect();
    assert_eq!(want.len(), 4);
    assert_eq!(session_rows(&session.stdout), want);
}

// The labelled table
// goes through the same renderer in a session as it does one-shot.
#[test]
fn ls_header_renders_identically_in_both_modes() {
    let env = env_with_vault();
    for title in ["AWS root account", "a", "Café"] {
        env.add("personal", title, &format!("v-{title}"));
    }

    let one_shot = env.cmd_ok(&["ls", "--header"]).stdout;
    let session = env.session_ok(&["use personal", PASSPHRASE, "ls --header", "exit"]);
    let want: Vec<&str> = one_shot.lines().collect();
    assert_eq!(want.len(), 5, "header + separator + 3 rows");
    assert_eq!(session_rows(&session.stdout), want);
}

// `search` in a session agrees with one-shot through the CLI too, and a
// second search in the session decrypts again rather than reading a cache
// (observable only as the same answer; the count is the library's test).
#[test]
fn search_renders_identically_in_both_modes() {
    let env = env_with_vault();
    search_fixture(&env);
    let one_shot = env.cmd_ok(&["search", "aws"]).stdout;
    let session = env.session_ok(&["search aws", PASSPHRASE, "search aws", "exit"]);
    let rows = session_rows(&session.stdout);
    let want: Vec<&str> = one_shot.lines().collect();
    assert_eq!(rows.len(), want.len() * 2, "{rows:?}");
    assert_eq!(rows[..want.len()], want[..]);
    assert_eq!(rows[want.len()..], want[..]);
}

// Search's and reindex's failures.
#[test]
fn no_search_or_reindex_failure_doubles_gages_own_prefix() {
    let env = env_with_vault();
    let doubled = format!("{0}{0}", crate::error::ERROR_PREFIX);
    for args in [
        &["search"][..],
        &["search", "a", "b"],
        &["search", "x", "--use", "nope"],
        &["reindex", "extra"],
        &["reindex", "--use", "nope"],
    ] {
        let r = env.run_with(args, &gage::prompter::FakePrompter::new());
        assert_ne!(r.code, Code::Success, "{args:?}");
        assert!(r.stderr.starts_with("gage: "), "{args:?}: {}", r.stderr);
        assert!(!r.stderr.contains(&doubled), "{args:?}: {}", r.stderr);
    }
    // In a session, too.
    let r = env.session_ok(&["search", "reindex --use nope", "exit"]);
    assert!(!r.stderr.contains(&doubled), "{}", r.stderr);
    assert_eq!(r.stderr.matches("gage: ").count(), 2, "{}", r.stderr);
}
