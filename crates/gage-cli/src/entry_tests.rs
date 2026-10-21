//! `insert`, `cat`, `rm` and `ls` through the real CLI, in-process, against
//! a fresh temp directory standing in for gage's roots. The Go row each
//! test ports is named above it. Real-binary properties (two processes
//! contending, a real terminal) live in `tests/`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gage::entry::{self, Entry};
use gage::exitcode::Code;
use gage::locker::Locker;
use gage::prompter::{FakePrompter, PromptError};
use gage::vault::Vault;
use gage::{gitrepo, vaultlock};
use uuid::Uuid;

use crate::commands::{
    align_cat_output, pad_key_lines, parse_field_flags, render_ls, render_ls_table, short_id,
    trim_one_trailing_newline,
};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};

/// The device name `Env` gives every vault it inits (its hostname,
/// normalized).
pub(crate) const DEVICE: &str = "test-host";

/// A prompter that unlocks once and, optionally, answers one value prompt.
pub(crate) fn unlocks() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

pub(crate) fn with_value(value: &str) -> FakePrompter {
    unlocks().with_value(value)
}

pub(crate) fn env_with_vault() -> Env {
    let env = Env::new();
    env.init("personal", &[]);
    env
}

impl Env {
    /// `gage insert <args>` answering the value prompt with `value`, and
    /// requiring success.
    pub(crate) fn insert_ok(&self, value: &str, args: &[&str]) -> Uuid {
        let mut full = vec!["insert"];
        full.extend_from_slice(args);
        let r = self.run_with(&full, &with_value(value));
        assert_eq!(r.code, Code::Success, "{full:?} failed:\n{}", r.stderr);
        inserted_id(&r)
    }

    pub(crate) fn cmd(&self, args: &[&str]) -> CliResult {
        self.run_with(args, &unlocks())
    }

    pub(crate) fn cmd_ok(&self, args: &[&str]) -> CliResult {
        let r = self.cmd(args);
        assert_eq!(r.code, Code::Success, "{args:?} failed:\n{}", r.stderr);
        r
    }

    pub(crate) fn vault(&self, name: &str) -> Vault {
        Vault::from_registration(&self.dirs, name, &self.entry(name)).unwrap()
    }

    /// Every entry in `name`, decrypted with the vault's real identity.
    pub(crate) fn entries(&self, name: &str) -> Vec<(Uuid, Entry)> {
        let vault = self.vault(name);
        let identity = vault.unlock(&unlocks()).unwrap();
        vault
            .entry_ids()
            .unwrap()
            .into_iter()
            .map(|id| (id, vault.read_entry(id, &identity).unwrap()))
            .collect()
    }

    pub(crate) fn only_entry(&self, name: &str) -> (Uuid, Entry) {
        let mut all = self.entries(name);
        assert_eq!(all.len(), 1, "want exactly one entry");
        all.remove(0)
    }

    pub(crate) fn commits(&self, name: &str) -> usize {
        gitrepo::commit_count(&self.entry(name).path).unwrap()
    }
}

/// The id in `gage: inserted "<title>" (<id>)`.
fn inserted_id(r: &CliResult) -> Uuid {
    let line = r.stdout.lines().next().unwrap();
    let id = line.rsplit_once('(').unwrap().1.trim_end_matches(')');
    Uuid::try_parse(id).unwrap_or_else(|_| panic!("no id in {line:?}"))
}

fn ls_lines(r: &CliResult) -> Vec<&str> {
    r.stdout.lines().collect()
}

// --- the one-shot handler and --use ----------------------------------------

// Go: TestNoUseFlagOperatesAgainstCurrentVault
#[test]
fn no_use_flag_operates_against_current_vault() {
    let env = Env::new();
    env.init("first", &[]);
    env.init("second", &[]);
    assert_eq!(env.global().current, "first");

    env.insert_ok("v", &["in-current"]);

    assert_eq!(env.entries("first").len(), 1);
    assert!(env.entries("second").is_empty());
    assert_eq!(ls_lines(&env.cmd_ok(&["ls"])).len(), 1);
}

// Go: TestUseFlagTargetsNamedVaultNotCurrent
#[test]
fn use_flag_targets_named_vault_not_current() {
    let env = Env::new();
    env.init("first", &[]);
    env.init("second", &[]);

    env.insert_ok("v", &["in-second", "--use", "second"]);
    env.insert_ok("v", &["also-second", "-u", "second"]);

    assert!(env.entries("first").is_empty());
    assert_eq!(env.entries("second").len(), 2);
    assert_eq!(env.cmd_ok(&["ls"]).stdout, "", "the current vault is empty");
    assert_eq!(ls_lines(&env.cmd_ok(&["ls", "--use", "second"])).len(), 2);
    let r = env.cmd_ok(&["cat", "in-second", "--use", "second"]);
    assert!(r.stdout.contains("in-second"));
    env.cmd_ok(&["rm", "in-second", "--use", "second"]);
    assert_eq!(env.entries("second").len(), 1);
}

// Go: TestUseUnregisteredVaultFailsBeforePromptingForAPassphrase. The
// prompter has nothing scripted, so any prompt at all would panic.
#[test]
fn use_unregistered_vault_fails_before_prompting_for_a_passphrase() {
    let env = env_with_vault();
    for args in [
        &["ls", "--use", "ghost"][..],
        &["cat", "x", "--use", "ghost"],
        &["rm", "x", "--use", "ghost"],
        &["insert", "x", "--use", "ghost"],
        &["show", "x", "--use", "ghost"],
        &["edit", "x", "--use", "ghost"],
        &["rename", "x", "y", "--use", "ghost"],
        &["generate", "x", "--use", "ghost"],
        &["insert", "x", "-e", "--use", "ghost"],
    ] {
        let prompter = FakePrompter::new();
        let r = env.run_with(args, &prompter);
        assert_eq!(r.code, Code::NotFound, "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains("ghost"), "{}", r.stderr);
        assert!(prompter.unlock_requests().is_empty(), "{args:?}");
        assert!(prompter.value_prompts().is_empty(), "{args:?}");
    }
}

#[test]
fn no_current_vault_is_a_usage_error_before_any_prompt() {
    let env = Env::new();
    let prompter = FakePrompter::new();
    let r = env.run_with(&["ls"], &prompter);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
}

/// Counts page locks and unlocks, to see an `Identity` close.
#[derive(Default)]
struct Counting {
    locks: AtomicUsize,
    unlocks: AtomicUsize,
}

impl Locker for Counting {
    fn lock(&self, _: &[u8]) -> std::io::Result<()> {
        self.locks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn unlock(&self, _: &[u8]) -> std::io::Result<()> {
        self.unlocks.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// Go: TestOneShotHandlerClosesIdentityOnEveryExitPathIncludingError
#[test]
fn one_shot_handler_closes_identity_on_every_exit_path_including_error() {
    let env = env_with_vault();
    let run = |result: Result<(), crate::error::CliError>| {
        let counter = Arc::new(Counting::default());
        let vault = env.vault("personal").with_locker(counter.clone());
        let got = crate::commands::with_unlocked(&vault, &unlocks(), |identity| {
            assert_eq!(counter.locks.load(Ordering::SeqCst), 1, "unlocked");
            assert_eq!(counter.unlocks.load(Ordering::SeqCst), 0, "still open");
            assert!(identity.page_locked());
            result
        });
        (
            got.is_ok(),
            counter.locks.load(Ordering::SeqCst),
            counter.unlocks.load(Ordering::SeqCst),
        )
    };

    assert_eq!(run(Ok(())), (true, 1, 1), "success path");
    let failure = crate::error::CliError::new(Code::Internal, "the command failed");
    assert_eq!(run(Err(failure)), (false, 1, 1), "error path");
}

// --- insert -----------------------------------------------------------------

// Go: TestInsertThenCatRoundTripsValueThroughTheCLI
#[test]
fn insert_then_cat_round_trips_value_through_the_cli() {
    let env = env_with_vault();

    let r = env.run_with(&["insert", "ProtonMail"], &with_value("hunter2"));

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let id = inserted_id(&r);
    assert_eq!(r.stdout, format!("gage: inserted \"ProtonMail\" ({id})\n"));
    let cat = env.cmd_ok(&["cat", "ProtonMail"]);
    let e = entry::unmarshal(cat.stdout.as_bytes()).unwrap();
    assert_eq!(e.title, "ProtonMail");
    assert_eq!(e.value, "hunter2");
    // By id, too, in any spelling.
    let by_id = env.cmd_ok(&["cat", &id.simple().to_string()]);
    assert_eq!(by_id.stdout, cat.stdout);
}

// Go: TestInsertDefaultPromptsForValueViaPrompter
#[test]
fn insert_default_prompts_for_value_via_prompter() {
    let env = env_with_vault();
    let prompter = with_value("from the prompter");

    let r = env.run_with(&["insert", "Bank"], &prompter);

    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(prompter.value_prompts(), vec!["Bank"]);
    assert_eq!(env.only_entry("personal").1.value, "from the prompter");
}

#[test]
fn a_failed_unlock_never_asks_for_the_value() {
    let env = env_with_vault();
    let prompter = FakePrompter::new().with_unlock_error(PromptError::Cancelled);

    let r = env.run_with(&["insert", "Bank"], &prompter);

    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(prompter.value_prompts().is_empty());
    assert!(env.entries("personal").is_empty());
}

// Go: TestInsertValueStdinTrimsExactlyOneNewline
#[test]
fn insert_value_stdin_trims_exactly_one_newline() {
    let env = env_with_vault();
    for (stdin, want) in [
        ("secret\n", "secret"),
        ("secret\r\n", "secret"),
        ("secret", "secret"),
        ("secret\n\n", "secret\n"),
        ("two\nlines\n", "two\nlines"),
        ("\n", ""),
    ] {
        let title = format!("t-{}", stdin.escape_debug());
        let r = env.run_stdin(
            &["insert", &title, "--value-stdin"],
            &unlocks(),
            stdin.as_bytes(),
        );
        assert_eq!(r.code, Code::Success, "{stdin:?}: {}", r.stderr);
        let entries = env.entries("personal");
        let e = entries.iter().find(|(_, e)| e.title == title).unwrap();
        assert_eq!(e.1.value, want, "{stdin:?}");
    }
}

// Go: TestInsertValueStdinTrimsOneTrailingNewlineAndRoundTrips
#[test]
fn insert_value_stdin_trims_one_trailing_newline_and_round_trips() {
    let env = env_with_vault();
    let prompter = unlocks();
    let r = env.run_stdin(
        &["insert", "Wifi", "--value-stdin"],
        &prompter,
        b"p@ss w0rd=\n",
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        prompter.value_prompts().is_empty(),
        "stdin replaces the prompt"
    );
    let cat = env.cmd_ok(&["cat", "Wifi"]);
    assert_eq!(
        entry::unmarshal(cat.stdout.as_bytes()).unwrap().value,
        "p@ss w0rd="
    );
}

// Go: TestValueStdinAndPromptPathsStoreIdenticalEntries
#[test]
fn value_stdin_and_prompt_paths_store_identical_entries() {
    let env = env_with_vault();
    let args = ["--description", "same", "--field", "user=me"];
    env.insert_ok("the value", &[&["via-prompt"][..], &args].concat());
    let mut full = vec!["insert", "via-stdin", "--value-stdin"];
    full.extend_from_slice(&args);
    let r = env.run_stdin(&full, &unlocks(), b"the value\n");
    assert_eq!(r.code, Code::Success, "{}", r.stderr);

    let entries = env.entries("personal");
    let get = |t: &str| &entries.iter().find(|(_, e)| e.title == t).unwrap().1;
    let (a, b) = (get("via-prompt"), get("via-stdin"));
    assert_eq!(a.value, b.value);
    assert_eq!(a.description, b.description);
    assert_eq!(a.fields, b.fields);
    assert_eq!(a.updated_by, b.updated_by);
}

// Go: TestTrimOneTrailingNewline
#[test]
fn trim_one_trailing_newline_cases() {
    let trim = |s: &str| trim_one_trailing_newline(s.to_string().into()).to_string();
    assert_eq!(trim(""), "");
    assert_eq!(trim("a"), "a");
    assert_eq!(trim("a\n"), "a");
    assert_eq!(trim("a\r\n"), "a");
    assert_eq!(trim("a\n\n"), "a\n");
    assert_eq!(trim("a\r\n\r\n"), "a\r\n");
    assert_eq!(trim("\n"), "");
    assert_eq!(trim("\r\n"), "");
    assert_eq!(trim("a\r"), "a\r", "a lone CR is not a line ending");
    assert_eq!(trim("a\nb"), "a\nb");
}

// Go: TestInsertMutuallyExclusiveValueFlagsRejectedBeforeAnyIO
#[test]
fn insert_mutually_exclusive_value_flags_rejected_before_any_io() {
    let env = env_with_vault();
    let before = env.commits("personal");
    // Nothing scripted: a prompt would panic, and so would reading stdin.
    let prompter = FakePrompter::new();

    let r = env.run_reader(
        &["insert", "x", "-m", "--value-stdin"],
        &prompter,
        &mut ExplodingReader,
    );

    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(prompter.unlock_requests().is_empty());
    assert!(prompter.value_prompts().is_empty());
    assert_eq!(env.commits("personal"), before);
    assert!(env.entries("personal").is_empty());

    // The guard above means something: either mode alone gets through.
    for mode in ["-m", "--value-stdin"] {
        let title = format!("alone{mode}");
        let r = env.run_stdin(&["insert", &title, mode], &unlocks(), b"v\n");
        assert_eq!(r.code, Code::Success, "{mode}: {}", r.stderr);
    }
}

/// Stdin that fails the test if anything reads it.
pub(crate) struct ExplodingReader;

impl std::io::Read for ExplodingReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("stdin was read");
    }
}

#[test]
fn a_cancelled_value_prompt_after_unlocking_writes_nothing() {
    let env = env_with_vault();
    let before = env.commits("personal");
    let prompter = unlocks().with_value_error(PromptError::Cancelled);

    let r = env.run_with(&["insert", "Bank"], &prompter);

    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert_eq!(r.stdout, "");
    assert_eq!(prompter.value_prompts(), vec!["Bank"]);
    assert!(env.entries("personal").is_empty());
    assert_eq!(env.commits("personal"), before);
}

#[test]
fn a_value_on_stdin_that_is_not_utf8_is_a_usage_error() {
    let env = env_with_vault();
    let before = env.commits("personal");
    for mode in ["--value-stdin", "-m"] {
        let r = env.run_stdin(&["insert", "x", mode], &unlocks(), b"\xff\xfe\n");
        assert_eq!(r.code, Code::Usage, "{mode}: {}", r.stderr);
        assert!(r.stderr.contains("UTF-8"), "{mode}: {}", r.stderr);
    }
    assert!(env.entries("personal").is_empty());
    assert_eq!(env.commits("personal"), before);
}

// Go: TestInsertDescriptionStoredAndShownByCat
#[test]
fn insert_description_stored_and_shown_by_cat() {
    let env = env_with_vault();
    env.insert_ok("v", &["Router", "--description", "the one in the hall"]);
    let cat = env.cmd_ok(&["cat", "Router"]);
    assert!(cat.stdout.contains("the one in the hall"), "{}", cat.stdout);
    assert_eq!(
        env.only_entry("personal").1.description,
        "the one in the hall"
    );
}

// Go: TestInsertDuplicateTitleRejectedUnlessForced
#[test]
fn insert_duplicate_title_rejected_unless_forced() {
    let env = env_with_vault();
    env.insert_ok("first", &["dup"]);
    let before = env.commits("personal");

    let refused = env.run_with(&["insert", "dup"], &with_value("second"));
    assert_eq!(refused.code, Code::Conflict, "{}", refused.stderr);
    assert!(refused.stderr.contains("--force"), "{}", refused.stderr);
    assert_eq!(env.entries("personal").len(), 1);
    assert_eq!(env.commits("personal"), before);

    env.insert_ok("second", &["dup", "-f"]);
    env.insert_ok("third", &["dup", "--force"]);
    assert_eq!(env.entries("personal").len(), 3);
}

// Go: TestInsertSetsTimestampsAndUpdatedBy
#[test]
fn insert_sets_timestamps_and_updated_by() {
    let env = env_with_vault();
    let start = now_unix();
    env.insert_ok("v", &["Stamped"]);
    let end = now_unix();

    let e = env.only_entry("personal").1;
    let (created, updated) = (e.created.unwrap(), e.updated.unwrap());
    assert_eq!(created, updated, "created and updated start equal");
    assert!((start..=end).contains(&created.as_unix()), "{created}");
    assert_eq!(e.updated_by, DEVICE);
}

// Go: TestInsertProducesExactlyOneCommit
#[test]
fn insert_produces_exactly_one_commit() {
    let env = env_with_vault();
    let before = env.commits("personal");
    env.insert_ok("v", &["one"]);
    assert_eq!(env.commits("personal"), before + 1);
    env.insert_ok("v", &["two"]);
    assert_eq!(env.commits("personal"), before + 2);
    assert!(gitrepo::is_clean(&env.entry("personal").path).unwrap());
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Today's UTC date as `YYYY-MM-DD`.
fn today() -> String {
    entry::Timestamp::from_unix(now_unix()).unwrap().to_string()[..10].to_string()
}

// Go: TestInsertCommitMessageIsUUIDOnly
#[test]
fn insert_commit_message_is_uuid_only() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["A Very Identifying Title"]);
    let head = gitrepo::head_commit(&env.entry("personal").path).unwrap();
    assert_eq!(
        head.message,
        id.to_string(),
        "no title, no verb, no newline"
    );
    assert!(env.only_entry("personal").0 == id);

    env.cmd_ok(&["rm", "A Very Identifying Title"]);
    let head = gitrepo::head_commit(&env.entry("personal").path).unwrap();
    assert_eq!(
        head.message,
        id.to_string(),
        "a removal is named the same way"
    );
}

/// Requires the tip commit of `vault` to be authored and committed by
/// gage itself.
fn assert_anonymous_author(vault: &std::path::Path, after: &str) {
    let head = gitrepo::head_commit(vault).unwrap();
    for (name, email) in [
        (&head.author_name, &head.author_email),
        (&head.committer_name, &head.committer_email),
    ] {
        assert_eq!(name, gitrepo::AUTHOR_NAME, "after {after}");
        assert_eq!(email, gitrepo::AUTHOR_EMAIL, "after {after}");
    }
}

// Go: TestCommitAuthorIgnoresTheUsersGitConfig. As Go: the vault's own
// `.git/config` names a real person, which is the config git consults
// first, and neither an insert nor a removal is authored as them.
#[test]
fn commit_author_ignores_the_users_git_config() {
    let env = env_with_vault();
    let vault = env.entry("personal").path;
    let config = vault.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[user]\n\tname = Real Human\n\temail = human@example.invalid\n");
    std::fs::write(&config, text).unwrap();

    env.insert_ok("v", &["Title"]);
    assert_anonymous_author(&vault, "insert");
    env.cmd_ok(&["rm", "Title"]);
    assert_anonymous_author(&vault, "rm");
}

const HOSTILE_HOME_ENV: &str = "GAGE_CLI_TEST_HOSTILE_HOME";

/// The child side of the test below: `init`, `insert` and `rm` in-process
/// with HOME pointing at a git config naming someone else.
#[test]
#[allow(clippy::disallowed_macros)]
fn hostile_home_trampoline() {
    if std::env::var_os(HOSTILE_HOME_ENV).is_none() {
        return;
    }
    let env = env_with_vault();
    let vault = env.entry("personal").path;
    env.insert_ok("v", &["Title"]);
    assert_anonymous_author(&vault, "insert");
    env.cmd_ok(&["rm", "Title"]);
    assert_anonymous_author(&vault, "rm");
    eprintln!("CHECKED");
}

// The global half of the row above: a user-wide git config (and git's
// author variables) naming someone else. A child process, because HOME is
// process-wide and libgit2 reads it once. This is the check that holds on
// a machine with no git identity of its own, where a fallback to gage's
// author would pass every other author test.
#[test]
fn commit_author_ignores_a_global_git_config() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join(".gitconfig"),
        "[user]\n\tname = Someone Else\n\temail = else@example.com\n",
    )
    .unwrap();

    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "entry_tests::hostile_home_trampoline",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HOSTILE_HOME_ENV, "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("GIT_AUTHOR_NAME", "Env Author")
        .env("GIT_COMMITTER_NAME", "Env Committer")
        .env_remove("GIT_CONFIG_GLOBAL")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "child failed:\n{stderr}");
    assert!(stderr.contains("CHECKED"), "child ran no check:\n{stderr}");
}

// --- cat / rm ---------------------------------------------------------------

// Go: TestCatUnknownFailsWithNotFound
#[test]
fn cat_unknown_fails_with_not_found() {
    let env = env_with_vault();
    env.insert_ok("v", &["exists"]);
    let r = env.cmd(&["cat", "nope"]);
    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("nope"), "{}", r.stderr);
    // A title substring resolves since P3 · M5; this one is in no title.
    assert_eq!(env.cmd(&["cat", "exis"]).code, Code::Success);
}

// Go: TestRmDeletesFileAndCommitsThenCatAndLsNoLongerShowIt
#[test]
fn rm_deletes_file_and_commits_then_cat_and_ls_no_longer_show_it() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["Doomed"]);
    env.insert_ok("v", &["Survivor"]);
    let before = env.commits("personal");

    let r = env.cmd_ok(&["rm", "Doomed"]);

    assert_eq!(r.stdout, format!("gage: removed {id}\n"));
    assert_eq!(env.commits("personal"), before + 1);
    assert!(
        !env.entry("personal")
            .path
            .join("entries")
            .join(format!("{id}.age"))
            .exists()
    );
    assert!(gitrepo::is_clean(&env.entry("personal").path).unwrap());
    assert_eq!(env.cmd(&["cat", "Doomed"]).code, Code::NotFound);
    let ls = env.cmd_ok(&["ls"]);
    assert!(!ls.stdout.contains("Doomed") && ls.stdout.contains("Survivor"));
    assert_eq!(env.cmd(&["rm", "Doomed"]).code, Code::NotFound);
}

#[test]
fn an_ambiguous_cat_or_rm_lists_the_candidates_and_exits_ambiguous() {
    let env = env_with_vault();
    let a = env.insert_ok("one", &["twin"]);
    let b = env.insert_ok("two", &["twin", "-f"]);
    let before = env.commits("personal");

    for cmd in ["cat", "rm"] {
        let r = env.cmd(&[cmd, "twin"]);
        assert_eq!(r.code, Code::Ambiguous, "{cmd}: {}", r.stderr);
        assert_eq!(r.stdout, "", "{cmd} wrote to stdout");
        assert!(r.stderr.contains(&a.to_string()), "{cmd}: {}", r.stderr);
        assert!(r.stderr.contains(&b.to_string()), "{cmd}: {}", r.stderr);
    }
    assert_eq!(env.entries("personal").len(), 2, "nothing was removed");
    assert_eq!(env.commits("personal"), before);
    // The id the error offers does resolve.
    env.cmd_ok(&["cat", &a.to_string()]);
    env.cmd_ok(&["rm", &a.to_string()]);
    assert_eq!(env.entries("personal").len(), 1);
}

// --- ls ---------------------------------------------------------------------

// Go: TestLsListsInsertedTitleAlongsidePartialUUID
#[test]
fn ls_lists_inserted_title_alongside_partial_uuid() {
    let env = env_with_vault();
    let id = env.insert_ok("v", &["ProtonMail"]);

    let r = env.cmd_ok(&["ls"]);

    let line = ls_lines(&r)[0].to_string();
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(cols[0], "ProtonMail");
    assert_eq!(cols[1], &id.to_string()[..8]);
    assert!(!line.contains(&id.to_string()), "the full id is not shown");
    assert_eq!(cols[2], today());
    assert_eq!(cols[3], today());
    assert_eq!(cols[4], DEVICE);
}

// Go: TestLsEmptyVaultNoOutput, and the ls half of
// TestLsAndSearchPrintNothingWhenThereIsNothing (the search half is in
// search_tests).
#[test]
fn ls_empty_vault_no_output() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["ls"]);
    assert_eq!(r.stdout, "");
    assert_eq!(r.stderr, "");
}

fn row(title: &str, created: Option<i64>, updated_by: &str) -> gage::entry::ListEntry {
    gage::entry::ListEntry {
        id: Uuid::parse_str("0123abcd-0000-4000-8000-000000000000").unwrap(),
        title: title.into(),
        description: String::new(),
        created: created.and_then(entry::Timestamp::from_unix),
        updated: None,
        updated_by: updated_by.into(),
    }
}

// Go: TestLsPlaceholdersForMissingColumns
#[test]
fn ls_placeholders_for_missing_columns() {
    let lines = render_ls(&[row("bare", None, "")]);
    assert_eq!(lines, ["bare  0123abcd  -  -  -"]);
    let lines = render_ls(&[row("full", Some(1_768_386_720), "laptop")]);
    assert_eq!(lines, ["full  0123abcd  2026-01-14  -  laptop"]);
    let lines = render_ls(&[row("", Some(0), "laptop")]);
    assert_eq!(
        lines,
        ["-  0123abcd  1970-01-01  -  laptop"],
        "an empty title"
    );
}

#[test]
fn ls_pads_titles_to_the_widest_counted_in_characters_and_trims_the_end() {
    let rows = [row("héllo", Some(0), "a"), row("x", Some(0), "")];
    let lines = render_ls(&rows);
    assert_eq!(
        lines,
        [
            "héllo  0123abcd  1970-01-01  -  a",
            "x      0123abcd  1970-01-01  -  -",
        ]
    );
    assert!(lines.iter().all(|l| l == l.trim_end()));
    assert!(render_ls(&[]).is_empty());
}

#[test]
fn ls_is_sorted_by_title_and_shows_the_real_writer() {
    let env = env_with_vault();
    for t in ["banana", "Apple", "cherry"] {
        env.insert_ok("v", &[t]);
    }
    let r = env.cmd_ok(&["ls"]);
    let titles: Vec<&str> = ls_lines(&r)
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(titles, ["Apple", "banana", "cherry"]);
}

// Go: TestLsDefaultOutputUnchangedByHeaderFeature. Plain `ls` is
// byte-for-byte what it was before `-H` existed.
#[test]
fn ls_default_output_unchanged_by_header_feature() {
    let env = env_with_vault();
    env.insert_ok("v", &["ProtonMail"]);

    let r = env.cmd_ok(&["ls"]);

    assert!(!r.stdout.contains('|'), "{:?}", r.stdout);
    assert_eq!(r.stdout.matches('\n').count(), 1, "{:?}", r.stdout);
    let id = env.only_entry("personal").0.to_string();
    assert_eq!(
        r.stdout,
        format!(
            "ProtonMail  {}  {}  {}  {DEVICE}\n",
            short_id(&id),
            today(),
            today()
        )
    );
}

// Go: TestLsHeaderPrintsLabelledPipeDelimitedTable
#[test]
fn ls_header_prints_labelled_pipe_delimited_table() {
    let env = env_with_vault();
    let full = env.insert_ok("v1", &["secret0"]).to_string();
    env.insert_ok("v2", &["test@gmail.com backup codes"]);

    for flag in ["--header", "-H"] {
        let r = env.cmd_ok(&["ls", flag]);
        let lines = ls_lines(&r);
        assert_eq!(lines.len(), 4, "header + separator + 2 rows:\n{}", r.stdout);
        let (header, sep, rows) = (lines[0], lines[1], &lines[2..]);

        let labels: Vec<&str> = header.split(" | ").map(str::trim_end).collect();
        assert_eq!(
            labels,
            ["title", "id", "created at", "updated at", "updated by"]
        );

        // Dashes and `+` only, as long as the header, broken under each `|`.
        assert!(sep.chars().all(|c| c == '-' || c == '+'), "{sep:?}");
        assert_eq!(sep.chars().count(), header.chars().count());
        for (i, c) in header.chars().enumerate() {
            if c == '|' {
                assert_eq!(sep.chars().nth(i), Some('+'), "{header:?}\n{sep:?}");
            }
        }

        let fields: Vec<&str> = rows[0].split(" | ").collect();
        assert_eq!(fields.len(), 5, "{:?}", rows[0]);
        assert_eq!(fields[0].trim_end(), "secret0");
        let id = fields[1].trim_end();
        assert!(full.starts_with(id) && id.len() < full.len(), "{id:?}");
        assert_eq!(fields[2].trim_end(), today());
        assert_eq!(fields[3].trim_end(), today());
        assert_eq!(fields[4], DEVICE, "the last column is never padded");
    }
}

// Not a Go row: the exact shape. Every column but the last is padded to its
// widest cell (header included), and the last column's dashes are sized by
// its label, since nothing follows it.
#[test]
fn ls_table_pads_all_but_the_last_column_and_sizes_its_dashes_from_the_label() {
    let rows = [
        row("héllo wörld", Some(0), "a-long-device-name"),
        row("x", Some(0), ""),
    ];
    assert_eq!(
        render_ls_table(&rows),
        [
            "title       | id       | created at | updated at | updated by",
            "------------+----------+------------+------------+-----------",
            "héllo wörld | 0123abcd | 1970-01-01 | -          | a-long-device-name",
            "x           | 0123abcd | 1970-01-01 | -          | -",
        ]
    );
    assert!(render_ls_table(&[]).is_empty(), "no rows, no table");
}

// Go: TestLsHeaderEmptyVaultNoOutput
#[test]
fn ls_header_empty_vault_no_output() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["ls", "--header"]);
    assert_eq!((r.stdout.as_str(), r.stderr.as_str()), ("", ""));
}

// Go: TestShortEntryIDLeavesShortStringsAlone
#[test]
fn short_entry_id_leaves_short_strings_alone() {
    assert_eq!(short_id("0123456789abcdef"), "01234567");
    assert_eq!(short_id("01234567"), "01234567");
    assert_eq!(short_id("0123"), "0123");
    assert_eq!(short_id(""), "");
    assert_eq!(short_id("ééééééééé"), "éééééééé", "counts characters");
}

// --- insert --field ---------------------------------------------------------

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// Go: TestInsertFieldMissingEqualsIsUsageErrorBeforeUnlock,
// TestInsertFieldEmptyNameIsUsageErrorBeforeUnlock and
// TestInsertFieldWhitespaceOnlyNameIsUsageErrorBeforeUnlock (the first
// loop), TestInsertDuplicateFieldNameIsUsageErrorBeforeUnlock and
// TestInsertFieldNamesEqualAfterStrippingAreDuplicates (the second),
// through the CLI: nothing scripted, so an unlock would panic. As Go's
// rows do, each case also checks the message says which mistake it was,
// and a duplicate is named after stripping.
#[test]
fn bad_field_flags_are_usage_errors_before_unlock() {
    let env = env_with_vault();
    let before = env.commits("personal");
    for (flag, why) in [
        ("username", "must be NAME=VALUE"),
        ("=value", "empty NAME"),
        ("   =value", "empty NAME"),
        ("\t \n=v", "empty NAME"),
    ] {
        let prompter = FakePrompter::new();
        let r = env.run_with(&["insert", "x", "--field", flag], &prompter);
        assert_eq!(r.code, Code::Usage, "{flag:?}: {}", r.stderr);
        assert!(prompter.unlock_requests().is_empty(), "{flag:?}");
        assert!(r.stderr.contains(why), "{flag:?}: {}", r.stderr);
    }
    for (pair, name) in [
        (["a=1", "a=2"], "a"),
        (["user=1", "u ser=2"], "user"),
        (["a=1", " a\t=1"], "a"),
    ] {
        let prompter = FakePrompter::new();
        let r = env.run_with(
            &["insert", "x", "--field", pair[0], "--field", pair[1]],
            &prompter,
        );
        assert_eq!(r.code, Code::Usage, "{pair:?}: {}", r.stderr);
        assert!(prompter.unlock_requests().is_empty(), "{pair:?}");
        let said = format!("{name:?} was given more than once");
        assert!(r.stderr.contains(&said), "{pair:?}: {}", r.stderr);
    }
    assert_eq!(env.commits("personal"), before);
}

// The parser behind the CLI rows above, on its own.
#[test]
fn parse_field_flags_rejects_what_it_must() {
    for bad in [&["noequals"][..], &["=v"], &[" \t=v"], &["a=1", "a=2"]] {
        let err = parse_field_flags(&strings(bad)).unwrap_err();
        assert_eq!(err.code(), Code::Usage, "{bad:?}");
    }
    assert!(parse_field_flags(&[]).unwrap().is_empty());
}

// Go: TestInsertFieldNameWhitespaceIsStripped
#[test]
fn insert_field_name_whitespace_is_stripped() {
    let f = parse_field_flags(&strings(&["  user name\t=me", "tot p=1"])).unwrap();
    assert_eq!(f["username"], "me");
    assert_eq!(f["totp"], "1");
}

// Go: TestInsertFieldValueWhitespaceIsPreserved
#[test]
fn insert_field_value_whitespace_is_preserved() {
    let f = parse_field_flags(&strings(&["note=  padded \t value  "])).unwrap();
    assert_eq!(f["note"], "  padded \t value  ");
}

// Go: TestInsertFieldValueContainingEqualsIsPreservedVerbatim
#[test]
fn insert_field_value_containing_equals_is_preserved_verbatim() {
    let f = parse_field_flags(&strings(&["token=a=b==c", "url=x?y=z"])).unwrap();
    assert_eq!(f["token"], "a=b==c");
    assert_eq!(f["url"], "x?y=z");
}

// Go: TestInsertFieldEmptyValueIsStored
#[test]
fn insert_field_empty_value_is_stored() {
    let env = env_with_vault();
    env.insert_ok("v", &["x", "--field", "pin="]);
    let e = env.only_entry("personal").1;
    assert_eq!(e.field("pin").unwrap(), "", "an empty value is a field");
}

// Go: TestInsertFieldWithMultilineSetsBothValueAndFields. The stdin here
// is not a terminal, so the "terminal" read is the same stream.
#[test]
fn insert_field_with_multiline_sets_both_value_and_fields() {
    let env = env_with_vault();
    let r = env.run_stdin(
        &["insert", "x", "-m", "--field", "user=me"],
        &unlocks(),
        b"line one\nline two\n",
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(r.stderr, "", "the Ctrl-D hint is for a terminal only");
    let e = env.only_entry("personal").1;
    assert_eq!(e.value, "line one\nline two\n", "-m keeps every byte");
    assert_eq!(e.field("user").unwrap(), "me");
}

// Go: TestInsertFieldWithValueStdinSetsBothValueAndFields
#[test]
fn insert_field_with_value_stdin_sets_both_value_and_fields() {
    let env = env_with_vault();
    let r = env.run_stdin(
        &["insert", "x", "--value-stdin", "--field", "user=me"],
        &unlocks(),
        b"s3cret\n",
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let e = env.only_entry("personal").1;
    assert_eq!(e.value, "s3cret");
    assert_eq!(e.field("user").unwrap(), "me");
}

// Go: TestInsertFieldWithoutValueModeStillPromptsOnceAndStoresFields
#[test]
fn insert_field_without_value_mode_still_prompts_once_and_stores_fields() {
    let env = env_with_vault();
    let prompter = with_value("prompted");
    let r = env.run_with(&["insert", "x", "--field", "user=me"], &prompter);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(prompter.value_prompts().len(), 1, "asked once");
    let e = env.only_entry("personal").1;
    assert_eq!(e.value, "prompted");
    assert_eq!(e.field("user").unwrap(), "me");
}

// Go: TestInsertFieldRoundTripsThroughShow and
// TestInsertMultipleFieldFlagsEachIndependentlyReadable, through the stored
// entry and `cat`; the `show --field` halves are in query_tests.
#[test]
fn insert_fields_round_trip_and_are_each_independently_readable() {
    let env = env_with_vault();
    env.insert_ok(
        "v",
        &[
            "Account",
            "--field",
            "username=me@proton.me",
            "--field",
            "totp=JBSW",
            "--field",
            "recovery code=1234 5678",
        ],
    );
    let e = env.only_entry("personal").1;
    assert_eq!(e.field("username").unwrap(), "me@proton.me");
    assert_eq!(e.field("totp").unwrap(), "JBSW");
    assert_eq!(e.field("recoverycode").unwrap(), "1234 5678");
    assert_eq!(e.fields.len(), 3);
    let cat = env.cmd_ok(&["cat", "Account"]);
    let shown = entry::unmarshal(cat.stdout.as_bytes()).unwrap();
    assert_eq!(shown.fields, e.fields);
}

// --- error prefix -----------------------------------------------------------

// Go: TestNoCommandDoublesGagesOwnPrefix, M4's rows: every failure the
// entry commands added, prefixed exactly once.
#[test]
fn no_entry_command_doubles_gages_own_prefix() {
    use crate::error::ERROR_PREFIX;
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    let env = env_with_vault();
    env.insert_ok("v", &["dup"]);
    env.insert_ok("v", &["twin"]);
    env.insert_ok("v", &["twin", "-f"]);
    let cases: &[(&str, &[&str])] = &[
        ("insert missing title", &["insert"]),
        (
            "insert unregistered vault",
            &["insert", "x", "--use", "ghost"],
        ),
        ("insert duplicate title", &["insert", "dup"]),
        (
            "insert field without =",
            &["insert", "x", "--field", "nope"],
        ),
        ("insert empty field name", &["insert", "x", "--field", "=v"]),
        (
            "insert duplicate field",
            &["insert", "x", "--field", "a=1", "--field", "a=2"],
        ),
        (
            "insert exclusive modes",
            &["insert", "x", "-m", "--value-stdin"],
        ),
        ("cat missing query", &["cat"]),
        ("cat unknown", &["cat", "nope"]),
        ("cat ambiguous", &["cat", "twin"]),
        ("cat unregistered vault", &["cat", "x", "--use", "ghost"]),
        ("rm unknown", &["rm", "nope"]),
        ("rm ambiguous", &["rm", "twin"]),
        ("ls unregistered vault", &["ls", "--use", "ghost"]),
        ("ls unknown flag", &["ls", "--no-such-flag"]),
    ];
    let mut results: Vec<(&str, CliResult)> = cases
        .iter()
        .map(|(name, args)| (*name, env.run_with(args, &with_value("v"))))
        .collect();
    // Failing at the passphrase, and with no vault to operate on.
    results.push((
        "ls cancelled passphrase",
        env.run_with(
            &["ls"],
            &FakePrompter::new().with_unlock_error(PromptError::Cancelled),
        ),
    ));
    results.push(("ls no current vault", Env::new().run(&["ls"])));
    for (name, r) in &results {
        assert_ne!(
            r.code,
            Code::Success,
            "{name}: this table only covers failures"
        );
        for line in r.stdout.lines().chain(r.stderr.lines()) {
            assert!(!line.contains(&doubled), "{name}: {line}");
            assert!(line.matches(ERROR_PREFIX).count() <= 1, "{name}: {line}");
        }
        assert!(
            r.stderr.starts_with(ERROR_PREFIX),
            "{name}: unprefixed:\n{}",
            r.stderr
        );
    }
}

// --- rows moved from P1 ------------------------------------------------------

// The insert half of Go's TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath
// (the `show` half is in query_tests, `identity list` is P5): a registration with
// an empty id fails `insert` before any path is built from it.
#[test]
fn a_registration_without_an_id_fails_insert_before_any_path_is_built() {
    let env = env_with_vault();
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id.clear();
    env.save_global(&global);
    let before = env.tree();
    let prompter = FakePrompter::new();

    let r = env.run_with(&["insert", "x"], &prompter);

    // The vault's own config still carries its id, so the empty one is
    // caught as a registration that doesn't match the vault, as Go's.
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("points at a different vault"),
        "{}",
        r.stderr
    );
    assert!(prompter.unlock_requests().is_empty());
    assert!(prompter.value_prompts().is_empty());
    assert_eq!(env.tree(), before, "no lock or identity path was created");
    assert!(env.entries_dir_is_empty("personal"));
}

impl Env {
    fn entries_dir_is_empty(&self, name: &str) -> bool {
        std::fs::read_dir(self.entry(name).path.join("entries"))
            .unwrap()
            .next()
            .is_none()
    }
}

const HOLDER_ENV: &str = "GAGE_CLI_TEST_LOCK_HOLDER";

/// The child side of the test below: takes the write lock at the given
/// path, announces it on stderr, and holds it until stdin closes.
#[test]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
fn lock_holder_trampoline() {
    use std::io::Read;
    let Some(path) = std::env::var_os(HOLDER_ENV) else {
        return;
    };
    let lock = vaultlock::Lock::acquire(
        std::path::Path::new(&path),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    eprintln!("ACQUIRED");
    let _ = std::io::stdin().read(&mut [0u8]);
    lock.release().unwrap();
}

// Go: TestTwoRegistrationsOfOneRepositoryShareOneLock. Two registrations
// of one repository, under different local names, contend on one lock: it
// is keyed by the vault's id, not the name. A child process holds the lock
// (a real process, since a thread-held lock would prove nothing about the
// file lock), and a real `insert` through the *other* registration has to
// wait for it, then succeeds once it lets go.
#[test]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
fn two_registrations_of_one_repository_share_one_lock() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    let env = env_with_vault();
    let mut global = env.global();
    let copy = global.vaults["personal"].clone();
    global.vaults.insert("alias".into(), copy);
    env.save_global(&global);
    assert_eq!(env.entry("alias").id, env.entry("personal").id);

    // Held through the "personal" registration's id.
    let lock_path = gage::paths::lock_file_path(&env.dirs, &env.entry("personal").id).unwrap();
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args([
            "entry_tests::lock_holder_trampoline",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HOLDER_ENV, &lock_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = BufReader::new(holder.stderr.take().unwrap());
    assert!(
        stderr.lines().any(|l| l.unwrap() == "ACQUIRED"),
        "the holder never took the lock"
    );

    let env_ref = &env;
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        scope.spawn(move || {
            let r = env_ref.run_with(&["insert", "via-alias", "--use", "alias"], &with_value("v"));
            let _ = tx.send(r.code);
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(500)).is_err(),
            "insert through the other registration did not wait for the lock"
        );
        drop(holder.stdin.take());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)),
            Ok(Code::Success),
            "insert never proceeded after the lock was released"
        );
    });
    let _ = holder.wait();
    assert_eq!(env.entries("personal").len(), 1);
}

// --- cat's display pass -----------------------------------------------------

const CAT_ID: &str = "0123abcd";

fn aligned(e: &Entry) -> String {
    let data = entry::marshal(e).unwrap();
    String::from_utf8(align_cat_output(CAT_ID, &data)).unwrap()
}

fn cat_lines(out: &str) -> Vec<&str> {
    out.trim_end_matches('\n').split('\n').collect()
}

fn colon(line: &str) -> usize {
    line.find(':')
        .unwrap_or_else(|| panic!("no colon in {line:?}"))
}

/// The one line of `lines` matching `pred`, failing unless exactly one does.
fn only_line(lines: &[&str], pred: impl Fn(&str) -> bool) -> usize {
    let hits: Vec<usize> = (0..lines.len()).filter(|&i| pred(lines[i])).collect();
    assert_eq!(hits.len(), 1, "want exactly one match in {lines:#?}");
    hits[0]
}

fn key_is(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with(':'))
}

fn simple(title: &str, value: &str) -> Entry {
    Entry {
        title: title.into(),
        value: value.into(),
        ..Entry::default()
    }
}

fn with_fields(mut e: Entry, fields: &[(&str, &str)]) -> Entry {
    e.fields = fields
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    e
}

// Go: TestAlignCatOutputInsertsIDLineAfterTitle
#[test]
fn align_cat_output_inserts_id_line_after_title() {
    let out = aligned(&simple("ProtonMail", "v"));
    let lines = cat_lines(&out);
    assert!(lines[0].starts_with("title"), "{lines:?}");
    let (key, value) = lines[1].split_once(':').unwrap();
    assert_eq!(key.trim_end(), "id", "{lines:?}");
    assert_eq!(value, format!(" {CAT_ID}"));
}

// Go: TestAlignCatOutputAlignsTopLevelColumn. The column follows the
// widest label present, not a fixed one.
#[test]
fn align_cat_output_aligns_top_level_column() {
    let mut described = simple("X", "v");
    described.description = "a description".into();
    for e in [
        simple("X", "v"),
        described,
        with_fields(simple("X", "v"), &[("username", "u"), ("totp_seed", "t")]),
    ] {
        let out = aligned(&e);
        let cols: Vec<usize> = cat_lines(&out)
            .into_iter()
            .filter(|l| !l.is_empty() && !l.starts_with(' '))
            .map(colon)
            .collect();
        assert!(cols.len() >= 3, "{out}");
        assert!(cols.iter().all(|&c| c == cols[0]), "misaligned:\n{out}");
    }
}

// Go: TestAlignCatOutputAlignsFieldsIndependently
#[test]
fn align_cat_output_aligns_fields_independently() {
    let e = with_fields(simple("X", "v"), &[("username", "u"), ("totp_seed", "t")]);
    let out = aligned(&e);
    let lines = cat_lines(&out);
    let cols: Vec<usize> = lines
        .iter()
        .filter(|l| l.starts_with(' '))
        .map(|l| colon(l))
        .collect();
    assert_eq!(cols.len(), 2, "{out}");
    assert_eq!(cols[0], cols[1], "{out}");
    let top = colon(lines[0]);
    assert_ne!(cols[0], top, "the fields column is its own:\n{out}");
    // "totp_seed" is the widest sub-key: indent 2 + 9.
    assert_eq!(cols[0], 2 + "totp_seed".len());
}

// Go: TestAlignCatOutputRoundTrips. Without the synthetic id line, what is
// printed parses back to the same entry.
#[test]
fn align_cat_output_round_trips() {
    let mut want = with_fields(
        simple("ProtonMail", "correcthorsebatterystaple"),
        &[
            ("username", "me@proton.me"),
            ("totp_seed", "JBSWY3DPEHPK3PXP"),
        ],
    );
    want.description = "Personal email".into();
    want.updated_by = "yubikey-5c-nfc-1".into();
    let out = aligned(&want);
    let stripped: String = out
        .split_inclusive('\n')
        .filter(|l| !key_is(l, "id"))
        .collect();
    assert_eq!(entry::unmarshal(stripped.as_bytes()).unwrap(), want);
}

// Go: TestAlignCatOutputDedentsMultilineValue
#[test]
fn align_cat_output_dedents_multiline_value() {
    let value = "line one\nline two\nline three";
    let out = aligned(&simple("X", value));
    let lines = cat_lines(&out);
    let at = only_line(&lines, |l| key_is(l, "value"));
    assert!(lines[at].contains('|'), "{lines:?}");
    assert_eq!(&lines[at + 1..], ["line one", "line two", "line three"]);
}

// Go: TestAlignCatOutputDedentsFieldsSubValue
#[test]
fn align_cat_output_dedents_fields_sub_value() {
    let e = with_fields(simple("X", "v"), &[("note", "a\nb\nc")]);
    let out = aligned(&e);
    let lines = cat_lines(&out);
    let at = only_line(&lines, |l| key_is(l, "note"));
    assert!(lines[at].contains('|'), "{lines:?}");
    assert_eq!(&lines[at + 1..], ["a", "b", "c"]);
}

// Go: TestAlignCatOutputDedentsValueFollowedByMoreKeys. The span ends at
// its own content: `fields` follows right after, found and aligned.
#[test]
fn align_cat_output_dedents_value_followed_by_more_keys() {
    let e = with_fields(simple("X", "alpha\nbeta"), &[("k", "v")]);
    let out = aligned(&e);
    let lines = cat_lines(&out);
    let value = only_line(&lines, |l| key_is(l, "value"));
    assert_eq!((lines[value + 1], lines[value + 2]), ("alpha", "beta"));
    let fields = only_line(&lines, |l| key_is(l, "fields"));
    assert_eq!(fields, value + 3, "{out}");
    assert_eq!(lines[fields + 1], "  k: v");
}

// Go: TestAlignCatOutputSkipsAlignmentForQuotedKeys
#[test]
fn align_cat_output_skips_alignment_for_quoted_keys() {
    let e = with_fields(
        simple("X", "v"),
        &[("weird: key", "val1"), ("username", "val2")],
    );
    let out = aligned(&e);
    assert!(
        out.contains("'weird: key': val1") || out.contains("\"weird: key\": val1"),
        "{out}"
    );
    // The quoted key doesn't widen the bare ones: "username" is alone.
    assert!(out.contains("  username: val2"), "{out}");
    let stripped: String = out
        .split_inclusive('\n')
        .filter(|l| !key_is(l, "id"))
        .collect();
    assert_eq!(entry::unmarshal(stripped.as_bytes()).unwrap(), e);
}

// Go: TestAlignCatOutputLeavesMultilineValueContentAlone. Content that
// looks like `key: value` is never padded or taken for a key, and its span
// does not bleed into the `fields` block after it.
#[test]
fn align_cat_output_leaves_multiline_value_content_alone() {
    let e = with_fields(
        simple("X", "note: this looks like yaml\nsecond line"),
        &[("username", "u")],
    );
    let out = aligned(&e);
    let lines = cat_lines(&out);
    let value = only_line(&lines, |l| key_is(l, "value"));
    assert_eq!(lines[value + 1], "note: this looks like yaml");
    assert_eq!(lines[value + 2], "second line");
    let fields = only_line(&lines, |l| key_is(l, "fields"));
    assert_eq!(fields, value + 3, "{out}");
    assert_eq!(lines[fields + 1], "  username: u");
}

// Not a Go row: `id` is a key in the column, even when it is the widest.
#[test]
fn align_cat_output_counts_the_id_line_toward_the_column() {
    let out = String::from_utf8(align_cat_output(CAT_ID, b"a: 1\nb: 2\n")).unwrap();
    assert_eq!(out, format!("a : 1\nid: {CAT_ID}\nb : 2\n"));
}

// Go: TestAlignCatOutputLeavesNonMappingInputAlone
#[test]
fn align_cat_output_leaves_non_mapping_input_alone() {
    for input in [
        "- just\n- a sequence\n",
        "a plain scalar\n",
        "title: [this is not\nvalid: yaml\n",
        "",
    ] {
        assert_eq!(align_cat_output(CAT_ID, input.as_bytes()), input.as_bytes());
    }
}

// Go: TestPadKeyLinesWithNoKeysIsANoOp
#[test]
fn pad_key_lines_with_no_keys_is_a_no_op() {
    let mut lines = vec!["one".to_string(), "two".to_string()];
    pad_key_lines(&mut lines, &[]);
    assert_eq!(lines, ["one", "two"]);
}

// Go: TestCatCommandPrintsIDLineThroughTheCLI
#[test]
fn cat_command_prints_id_line_through_the_cli() {
    let env = env_with_vault();
    let id = env.insert_ok("secret", &["ProtonMail"]);
    let want = short_id(&id.to_string()).to_string();

    let cat = env.cmd_ok(&["cat", "ProtonMail"]);

    let id_line = cat
        .stdout
        .lines()
        .find(|l| key_is(l, "id"))
        .unwrap_or_else(|| panic!("no id line:\n{}", cat.stdout));
    assert_eq!(id_line.split_once(':').unwrap().1.trim(), want);
    // `ls` shows the same id for the entry.
    assert!(env.cmd_ok(&["ls"]).stdout.contains(&want));
    let stripped: String = cat
        .stdout
        .split_inclusive('\n')
        .filter(|l| !key_is(l, "id"))
        .collect();
    let e = entry::unmarshal(stripped.as_bytes()).unwrap();
    assert_eq!(
        (e.title.as_str(), e.value.as_str()),
        ("ProtonMail", "secret")
    );
}

// Beyond Go's rows: M12b's commands keep Go's descriptions and flags (a
// reference, not a byte gate: Q-COMPAT).
#[test]
fn p6_ls_cat_and_vault_remove_have_their_go_fixture_flags_and_descriptions() {
    let env = Env::new();
    for (args, fixture) in [
        (
            &["ls", "--help"][..],
            include_str!("../../../tests/fixtures/go/help/ls.txt"),
        ),
        (
            &["cat", "--help"][..],
            include_str!("../../../tests/fixtures/go/help/cat.txt"),
        ),
        (
            &["vault", "remove", "--help"][..],
            include_str!("../../../tests/fixtures/go/help/vault_remove.txt"),
        ),
    ] {
        let help = env.ok(args).stdout;
        let name = args[..args.len() - 1].join(" ");
        assert!(
            help.starts_with(fixture.lines().next().unwrap()),
            "{name}:\n{help}"
        );
        for line in fixture
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("Usage:"))
            .filter(|l| !l.is_empty())
        {
            assert!(help.contains(line.trim()), "{name}: {line:?}\n{help}");
        }
        for line in fixture.lines().filter(|l| l.trim_start().starts_with('-')) {
            let text = line.split("   ").last().unwrap().trim();
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
    }
    // `ls` alone gains `-H/--header`; plain `cat` takes no new flag.
    let ls = env.ok(&["ls", "--help"]).stdout;
    assert!(ls.contains("-H, --header"), "{ls}");
    assert!(!env.ok(&["cat", "--help"]).stdout.contains("--header"));
}
