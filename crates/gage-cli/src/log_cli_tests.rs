//! `gage log` and `gage history --decrypt` through the real CLI,
//! in-process.

use crate::exitcode::ExitCode;
use gage::entry::Entry;
use gage::prompter::FakePrompter;
use uuid::Uuid;

use crate::entry_tests::{env_with_vault, unlocks};
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::repl_tests::passphrase_prompts;

/// Rewrites `id`'s entry through the library, which commits one revision.
fn edit(env: &Env, id: Uuid, change: impl FnOnce(&mut Entry)) {
    let vault = env.vault("personal");
    let identity = vault.unlock(&unlocks()).unwrap();
    let mut e = vault.read_entry(id, &identity).unwrap();
    change(&mut e);
    vault.update(id, &e, &FakePrompter::new()).unwrap();
}

/// An entry with a title, a value and a field, edited once, so every
/// plaintext a leak could come from is present.
fn secret_entry(env: &Env) -> Uuid {
    let id = env.insert_ok(
        "first-secret-value",
        &["--field", "pin=424242", "Hidden Title"],
    );
    edit(env, id, |e| e.value = "second-secret-value".into());
    id
}

fn short_hashes(env: &Env, id: Uuid) -> Vec<String> {
    env.vault("personal")
        .log(id)
        .unwrap()
        .iter()
        .map(|l| l.hash[..7].to_string())
        .collect()
}

/// `2026-01-14 10:32:00 +0000  abc1234`, optionally ` (deleted)`.
fn is_log_line(line: &str) -> bool {
    let line = line.strip_suffix(" (deleted)").unwrap_or(line);
    let b = line.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    line.len() >= 27
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .all(|&i| digit(i))
        && &line[4..5] == "-"
        && &line[10..11] == " "
        && &line[19..20] == " "
        && matches!(&line[20..21], "+" | "-")
        && line[21..25].bytes().all(|c| c.is_ascii_digit())
        && &line[25..27] == "  "
}

// The title resolves the
// query, so it is typed; nothing decrypted comes back out.
#[test]
fn log_shows_timestamps_and_no_decrypted_content() {
    let env = env_with_vault();
    let id = secret_entry(&env);
    let hashes = short_hashes(&env, id);

    let r = env.cmd_ok(&["log", "Hidden Title"]);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", r.stdout);
    for (line, hash) in lines.iter().zip(&hashes) {
        assert!(is_log_line(line), "{line:?}");
        assert!(line.ends_with(&format!("  {hash}")), "{line:?} / {hash}");
    }
    let shown = format!("{}{}", r.stdout, r.stderr);
    for secret in [
        "first-secret-value",
        "second-secret-value",
        "424242",
        "pin",
        "Hidden Title",
    ] {
        assert!(!shown.contains(secret), "{secret:?} leaked:\n{shown}");
    }
}

// Commit messages are UUIDs;
// the output prints neither them nor any title.
#[test]
fn log_reveals_no_titles_from_commit_messages() {
    let env = env_with_vault();
    let id = secret_entry(&env);
    env.insert_ok("v", &["Another Title"]);

    for args in [&["log", "Hidden Title"][..], &["log"][..]] {
        let r = env.cmd_ok(args);
        let shown = format!("{}{}", r.stdout, r.stderr);
        assert!(!shown.contains(&id.to_string()), "{args:?}:\n{shown}");
        assert!(!shown.contains("Title"), "{args:?}:\n{shown}");
    }
}

// After `rm` the title no longer
// resolves, but the full id does.
#[test]
fn log_shows_a_deleted_entrys_end() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Doomed"]);
    env.cmd_ok(&["rm", "Doomed"]);

    let r = env.cmd_ok(&["log", &id.to_string()]);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", r.stdout);
    assert!(lines[0].ends_with(" (deleted)"), "{}", lines[0]);
    assert!(!lines[1].contains("deleted"), "{}", lines[1]);
    // The simple (hyphenless) spelling is a full id too.
    assert_eq!(
        env.cmd_ok(&["log", &id.simple().to_string()]).stdout,
        r.stdout
    );
}

// A substring of a removed
// entry's id is not an id, and a title still resolves title-first.
#[test]
fn log_on_a_query_that_is_neither_a_title_nor_a_uuid() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Doomed"]);
    env.cmd_ok(&["rm", "Doomed"]);

    for query in ["nonsense", &id.to_string()[..8]] {
        let r = env.cmd(&["log", query]);
        assert_eq!(r.code, ExitCode::NotFound, "{query}: {}", r.stderr);
        assert_eq!(r.stdout, "");
    }
    let r = env.cmd(&["history", "--decrypt", "nonsense"]);
    assert_eq!(r.code, ExitCode::NotFound, "{}", r.stderr);
}

#[test]
fn log_and_history_on_an_entry_with_no_history_say_nothing_is_there() {
    let env = env_with_vault();
    let never = Uuid::new_v4().to_string();
    for args in [
        &["log", never.as_str()][..],
        &["history", "--decrypt", never.as_str()][..],
    ] {
        let r = env.cmd_ok(args);
        assert_eq!(r.stdout, "", "{args:?}");
        assert!(r.stderr.contains("no history"), "{args:?}: {}", r.stderr);
    }
}

// One unlock serves both commands, and
// the history file records what was typed, never what was decrypted.
#[test]
fn log_and_history_work_in_a_session() {
    let env = env_with_vault();
    secret_entry(&env);

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "log Hidden",
        "history --decrypt Hidden",
        "exit",
    ]);
    assert_eq!(passphrase_prompts(&r), 1, "{}", r.stderr);
    // The prompt shares the first output line with the command's output.
    let logged = r
        .stdout
        .lines()
        .filter(|l| is_log_line(l.rsplit("gage> ").next().unwrap()));
    assert_eq!(logged.count(), 2, "{}", r.stdout);
    assert!(r.stdout.contains("first-secret-value"), "{}", r.stdout);
    assert!(r.stdout.contains("second-secret-value"), "{}", r.stdout);

    // Nothing `history` decrypted reaches the history file.
    let history = std::fs::read_to_string(env.dirs.state.join("history")).unwrap();
    assert_eq!(
        history,
        "use personal\nlog Hidden\nhistory --decrypt Hidden\nexit\n"
    );
}

// A vault-wide log tags each line with its entry's short id,
// and covers every entry.
#[test]
fn a_vault_wide_log_tags_each_line_with_its_entry() {
    let env = env_with_vault();
    let a = env.insert_ok("v", &["Alpha"]);
    let b = env.insert_ok("v", &["Beta"]);
    edit(&env, a, |e| e.value = "changed".into());

    let r = env.cmd_ok(&["log"]);
    let lines: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", r.stdout);
    let count = |id: Uuid| {
        let tag = format!("  {}", &id.to_string()[..8]);
        lines.iter().filter(|l| l.contains(&tag)).count()
    };
    assert_eq!((count(a), count(b)), (2, 1), "{}", r.stdout);
    assert!(lines.iter().all(|l| is_log_line(l)), "{}", r.stdout);
}

#[test]
fn a_vault_wide_log_of_an_empty_vault_says_nothing_is_there() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["log"]);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("no history"), "{}", r.stderr);
}

// Refused before anything is unlocked.
#[test]
fn history_requires_decrypt() {
    let env = env_with_vault();
    env.insert_ok("v", &["Account"]);
    let prompter = unlocks();
    let r = env.run_with(&["history", "Account"], &prompter);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--decrypt"), "{}", r.stderr);
    assert!(r.stderr.contains("gage log"), "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
}

// No bulk form.
#[test]
fn history_with_no_query_is_a_usage_error() {
    let env = env_with_vault();
    let prompter = unlocks();
    let r = env.run_with(&["history", "--decrypt"], &prompter);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert_eq!(env.cmd(&["history"]).code, ExitCode::Usage);
}

// Oldest first; the first
// revision is all additions, the next shows only what changed.
#[test]
fn history_decrypt_walks_revisions_and_diffs() {
    let env = env_with_vault();
    let id = secret_entry(&env);
    let hashes = short_hashes(&env, id);

    let r = env.cmd_ok(&["history", "--decrypt", "Hidden Title"]);
    let headers: Vec<&str> = r.stdout.lines().filter(|l| l.starts_with("== ")).collect();
    assert_eq!(headers.len(), 2, "{}", r.stdout);
    assert!(
        headers[0].ends_with(&format!("  {}", hashes[1])),
        "{headers:?}"
    );
    assert!(
        headers[1].ends_with(&format!("  {}", hashes[0])),
        "{headers:?}"
    );

    let (first, second) = r.stdout.split_once(headers[1]).unwrap();
    assert!(first.contains("+ value: first-secret-value"), "{first}");
    assert!(first.contains("+ title: Hidden Title"), "{first}");
    assert!(first.contains("+ fields.pin: 424242"), "{first}");
    assert!(second.contains("- value: first-secret-value"), "{second}");
    assert!(second.contains("+ value: second-secret-value"), "{second}");
    assert!(second.contains("  title: Hidden Title"), "{second}");
    assert!(second.contains("  fields.pin: 424242"), "{second}");
}

#[test]
fn history_decrypt_marks_a_description_change() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Account"]);
    edit(&env, id, |e| e.description = "now described".into());

    let r = env.cmd_ok(&["history", "--decrypt", "Account"]);
    assert!(
        r.stdout.contains("+ description: now described"),
        "{}",
        r.stdout
    );
    // Neither revision's value changed.
    assert!(!r.stdout.contains("- value"), "{}", r.stdout);
}

#[test]
fn history_decrypt_shows_a_field_that_was_removed() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["--field", "pin=9999", "Account"]);
    edit(&env, id, |e| {
        e.fields.clear();
    });

    let r = env.cmd_ok(&["history", "--decrypt", "Account"]);
    assert!(r.stdout.contains("- fields.pin: 9999"), "{}", r.stdout);
}

#[test]
fn history_decrypt_shows_a_deleted_revision() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Doomed"]);
    env.cmd_ok(&["rm", "Doomed"]);

    let r = env.cmd_ok(&["history", "--decrypt", &id.to_string()]);
    let headers: Vec<&str> = r.stdout.lines().filter(|l| l.starts_with("== ")).collect();
    assert_eq!(headers.len(), 2, "{}", r.stdout);
    assert!(headers[1].ends_with(" (deleted)"), "{headers:?}");
    assert!(!headers[0].contains("deleted"), "{headers:?}");
}

// A revision this identity can't read fails the command: the output never
// looks complete when it isn't.
#[test]
fn history_decrypt_fails_on_an_undecryptable_revision() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Account"]);
    // A damaged middle revision (not a wrong key, but the same obligation:
    // an error, not a skipped revision). The newest is sound, so the title
    // still resolves.
    let vault = env.vault("personal");
    let file = vault.path.join(format!("entries/{id}.age"));
    let sound = std::fs::read(&file).unwrap();
    std::fs::write(&file, b"not age").unwrap();
    gage::gitrepo::commit_all(&vault.path, &id.to_string()).unwrap();
    std::fs::write(&file, sound).unwrap();
    gage::gitrepo::commit_all(&vault.path, &id.to_string()).unwrap();

    let r = env.cmd(&["history", "--decrypt", "Account"]);
    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stdout);
    assert!(r.stderr.contains(&id.to_string()), "{}", r.stderr);
    assert!(
        !r.stdout.contains("== "),
        "no partial output:\n{}",
        r.stdout
    );
}
