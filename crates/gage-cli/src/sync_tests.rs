//! `gage pull`, `push`, `sync`, `clone` and `init --remote` through the
//! real CLI, in-process, against bare repositories standing in for the
//! remote and a [`Device`] for the other machine: Go's `sync_cli_test.go`,
//! `clone_test.go` and `init_remote_auth_test.go`. Each test names its Go
//! row.

use std::path::{Path, PathBuf};

use gage::exitcode::Code;
use gage::gittest::{self, Answer, BareRemote, Device, HttpRecorder};
use gage::prompter::FakePrompter;
use gage::remotesync::SyncReport;
use gage::syncresolve::{Resolution, entry_count};
use gage::vault::Vault;
use gage::{gitrepo, paths, remoteauth, vaultconfig};

use crate::commands::{report_conflicts, sync_line};
use crate::editor::fake::{Action, FakeEditor, Set};
use crate::entry_tests::{unlocks, with_value};
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::prompter::{SecretInput, TerminalPrompter};
use crate::registry::registry;
use crate::repl_tests::passphrase_prompts;

impl Env {
    /// A real `gage init <name> --remote <fresh bare repo>`: a vault that
    /// exists on both sides (Go's `initVaultWithRemote`). Nothing pushes by
    /// hand: init publishing its own first commit is part of what a remote
    /// at creation means.
    fn init_with_remote(&self, name: &str) -> (PathBuf, BareRemote) {
        let remote = gittest::new_bare_remote();
        let r = self.run(&["init", name, "--remote", url(&remote), "--no-recovery-key"]);
        assert_eq!(r.code, Code::Success, "init: {}", r.stderr);
        (self.entry(name).path, remote)
    }
}

fn url(remote: &BareRemote) -> &str {
    remote.path().to_str().unwrap()
}

/// Commits a file in the vault the way something outside gage would.
fn local_commit(vault: &Path, rel: &str, content: &str) {
    std::fs::write(vault.join(rel), content).unwrap();
    gitrepo::commit_all(vault, "a local commit").unwrap();
}

fn entry_files(dir: &Path) -> Vec<String> {
    match std::fs::read_dir(dir.join("entries")) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// --- init --remote ----------------------------------------------------------

// Go: TestInitPublishesTheNewVault
#[test]
fn init_publishes_the_new_vault() {
    let env = Env::new();
    let remote = gittest::new_bare_remote();
    let r = env.run(&[
        "init",
        "personal",
        "--remote",
        url(&remote),
        "--no-recovery-key",
    ]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("published"), "{}", r.stdout);
    assert!(Device::clone(remote.path()).exists(".age-recipients"));
    assert_eq!(env.entry("personal").git.origin, url(&remote));
}

// Go: TestInitAgainstAMissingRepositorySaysToCreateIt. gage never creates
// a repository for you, and says so; the local vault stays.
#[test]
fn init_against_a_missing_repository_says_to_create_it() {
    let env = Env::new();
    let missing = env.root.path().join("nothing-here.git");
    let r = env.run(&[
        "init",
        "personal",
        "--remote",
        missing.to_str().unwrap(),
        "--no-recovery-key",
    ]);
    assert_ne!(r.code, Code::Success, "{}", r.stdout);
    assert!(r.stderr.contains("create an empty one"), "{}", r.stderr);
    assert!(r.stderr.contains("gage push"), "{}", r.stderr);
    assert!(env.entry("personal").path.join(".age-recipients").exists());
    assert!(!missing.exists(), "gage created the repository");
}

// The same through the auth bucket: a host answering 404, which is how a
// host says a repository isn't there.
#[test]
fn init_against_a_host_without_the_repository_says_to_create_it() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Status(404));
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &unlocks().with_value(""),
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("create an empty one"), "{}", r.stderr);
    assert!(env.global().vaults.contains_key("personal"));
}

// Offline at creation is not a failure: the next sync publishes it.
#[test]
fn init_against_an_unreachable_remote_succeeds_and_says_so() {
    let env = Env::new();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--remote",
            &gittest::refused_url(),
            "--no-recovery-key",
        ],
        &unlocks().with_value(""),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stderr.contains("could not reach"), "{}", r.stderr);
    assert!(r.stderr.contains("next successful sync"), "{}", r.stderr);
}

const HOST: &str = "127.0.0.1";

// Go: TestInitSolicitsATokenWhenNoneIsStored. Checked at the recorder: the
// token typed at the prompt is what the push presented.
#[test]
fn init_solicits_a_token_when_none_is_stored() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Challenge);
    let p = unlocks().with_value("tok_from_prompt");
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &p,
    );
    assert_ne!(r.code, Code::Success, "the recorder refuses every push");

    let prompts = p.value_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains(HOST), "{prompts:?}");
    assert_eq!(
        *remoteauth::load(&env.dirs, HOST).unwrap(),
        "tok_from_prompt"
    );
    assert!(
        recorder
            .authorizations()
            .contains(&gittest::basic("gage", "tok_from_prompt")),
        "the push never presented the solicited token: {:?}",
        recorder.requests()
    );
    assert!(!r.stdout.contains("tok_from_prompt") && !r.stderr.contains("tok_from_prompt"));
}

// Go: TestInitSkipsTheTokenWhenTheAnswerIsBlank. Not every remote needs a
// token; a blank answer stores nothing and the push goes anonymously.
#[test]
fn init_skips_the_token_when_the_answer_is_blank() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Challenge);
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &unlocks().with_value(""),
    );
    assert_ne!(r.code, Code::Success);
    assert!(matches!(
        remoteauth::load(&env.dirs, HOST),
        Err(remoteauth::Error::NoToken { .. })
    ));
    assert!(!recorder.requests().is_empty(), "the push never happened");
    assert!(recorder.authorizations().is_empty(), "not anonymous");
}

// Go: TestInitDoesNotPromptWhenATokenAlreadyExists. The FakePrompter has
// no value scripted, so a prompt would fail the test outright.
#[test]
fn init_does_not_prompt_when_a_token_already_exists() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Challenge);
    remoteauth::store(&env.dirs, HOST, "tok_existing").unwrap();
    let p = unlocks();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &p,
    );
    assert_ne!(r.code, Code::Success);
    assert!(p.value_prompts().is_empty(), "{:?}", p.value_prompts());
    assert!(
        recorder
            .authorizations()
            .contains(&gittest::basic("gage", "tok_existing"))
    );
}

// Go: TestInitDoesNotPromptForALocalRemote
#[test]
fn init_does_not_prompt_for_a_local_remote() {
    let env = Env::new();
    let remote = gittest::new_bare_remote();
    let p = unlocks();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--remote",
            url(&remote),
            "--no-recovery-key",
        ],
        &p,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(p.value_prompts().is_empty());
    assert!(!env.dirs.state.join("tokens").exists());
}

// --- pull / push / sync -----------------------------------------------------

// Go: TestPushPublishesAndPullCatchesUp
#[test]
fn push_publishes_and_pull_catches_up() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("notes.txt", "from elsewhere", "theirs");

    let r = env.run(&["pull"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("caught"), "{}", r.stdout);
    assert!(vault.join("notes.txt").exists());

    local_commit(&vault, "mine.txt", "mine");
    let r = env.run(&["push"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("pushed"), "{}", r.stdout);
    assert!(Device::clone(remote.path()).exists("mine.txt"));
}

// Go: TestOneShotCommandPullsOnItsImplicitUnlock. `show` knows nothing of
// remotes, but it unlocks, and unlocking pulls.
#[test]
fn one_shot_command_pulls_on_its_implicit_unlock() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    env.insert_ok("hunter2", &["ProtonMail"]);
    Device::clone(remote.path()).write_commit_push("notes.txt", "from elsewhere", "theirs");

    let r = env.cmd(&["show", "ProtonMail"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(r.stdout, "hunter2\n");
    assert!(
        vault.join("notes.txt").exists(),
        "show's unlock did not pull"
    );
}

// Go: TestWriteReachesTheRemoteWithoutAnyoneAskingIt
#[test]
fn write_reaches_the_remote_without_anyone_asking_it() {
    let env = Env::new();
    let (_, remote) = env.init_with_remote("personal");
    // Warnings go to the prompter (a FakePrompter records them rather than
    // printing them), so that is where friction would show.
    let p = with_value("hunter2");
    let r = env.run_with(&["insert", "ProtonMail"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    assert!(
        r.stderr.is_empty(),
        "sync friction on a clean write: {}",
        r.stderr
    );
    assert_eq!(entry_files(Device::clone(remote.path()).path()).len(), 1);
}

// Beyond Go's one row: every write command pushes.
#[test]
fn every_write_command_pushes() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    // Commits only ever grow, one per write, so a fresh clone of the
    // remote matching the vault's count is the write having arrived.
    let remote_commits = || gitrepo::commit_count(Device::clone(remote.path()).path()).unwrap();
    let local_commits = || gitrepo::commit_count(&vault).unwrap();
    let editor = FakeEditor::new(Action::Set(&[Set::Value("rotated")]));
    env.set_editor(&editor);
    let cases: &[(&str, &[&str], FakePrompter)] = &[
        ("insert", &["insert", "a"], with_value("v")),
        ("generate", &["generate", "b"], unlocks()),
        ("edit", &["edit", "a"], unlocks()),
        ("rename", &["rename", "a", "c"], unlocks()),
        ("rm", &["rm", "c"], unlocks()),
    ];
    for (name, args, p) in cases {
        let before = local_commits();
        let r = env.run_with(args, p);
        assert_eq!(r.code, Code::Success, "{name}: {}", r.stderr);
        assert!(p.warnings().is_empty(), "{name}: {:?}", p.warnings());
        assert_eq!(local_commits(), before + 1, "{name} committed once");
        assert_eq!(remote_commits(), local_commits(), "{name} did not push");
    }
}

// Go: TestPullReportsADivergenceRatherThanMergingIt
#[test]
fn pull_reports_a_divergence_rather_than_merging_it() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("notes.txt", "from elsewhere", "theirs");
    local_commit(&vault, "local.txt", "local");

    let r = env.run(&["pull"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("diverged"), "{}", r.stderr);
    assert!(!vault.join("notes.txt").exists(), "pull merged");
}

// Go: TestSyncMergesADisjointDivergenceWithoutPrompting. Sync unlocks
// lazily: nothing here is decrypted, so nothing asks for a passphrase.
#[test]
fn sync_merges_a_disjoint_divergence_without_prompting() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("notes.txt", "from elsewhere", "theirs");
    local_commit(&vault, "local.txt", "local");

    let p = FakePrompter::new();
    let r = env.run_with(&["sync"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("merged"), "{}", r.stdout);
    assert!(vault.join("notes.txt").exists());
    assert!(p.unlock_requests().is_empty(), "a mergeable sync prompted");
    let fresh = Device::clone(remote.path());
    assert!(fresh.exists("local.txt") && fresh.exists("notes.txt"));
}

// Go: TestSyncReportsAConflictingDivergenceAndNamesTheVault
#[test]
fn sync_reports_a_conflicting_divergence_and_names_the_vault() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("shared.txt", "their version", "theirs");
    local_commit(&vault, "shared.txt", "my version");

    let r = env.run(&["sync"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("shared.txt"), "{}", r.stderr);
    assert!(r.stderr.contains(vault.to_str().unwrap()), "{}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(vault.join("shared.txt")).unwrap(),
        "my version"
    );
}

// Go: TestSyncOnAVaultWithNoRemoteSaysSoAndSucceeds. Not "in sync with
// origin": there is no origin.
#[test]
fn sync_on_a_vault_with_no_remote_says_so_and_succeeds() {
    let env = Env::new();
    env.init("personal", &[]);
    for verb in ["sync", "pull", "push"] {
        let r = env.run(&[verb]);
        assert_eq!(r.code, Code::Success, "{verb}: {}", r.stderr);
        assert!(r.stdout.contains("local-only"), "{verb}: {}", r.stdout);
        assert!(r.stdout.contains("no remote"), "{verb}: {}", r.stdout);
    }
}

// Go: TestOfflineExitsUnreachableAndDivergedExitsConflict. The same failed
// push needs different things from a script: retry later, or a human.
#[test]
fn offline_exits_unreachable_and_diverged_exits_conflict() {
    let env = Env::new();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--remote",
            &gittest::refused_url(),
            "--no-recovery-key",
        ],
        &unlocks().with_value(""),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    for verb in ["push", "pull", "sync"] {
        let r = env.run(&[verb]);
        assert_eq!(r.code, Code::Unreachable, "{verb}: {}", r.stderr);
    }

    let diverged = Env::new();
    let (vault, remote) = diverged.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("shared.txt", "theirs", "theirs");
    local_commit(&vault, "shared.txt", "mine");
    assert_eq!(diverged.run(&["push"]).code, Code::Conflict);
}

// Auth is its own exit code, and the message names the host and the fix.
#[test]
fn an_auth_failure_exits_locked_or_auth_with_the_hint() {
    let env = Env::new();
    let recorder = HttpRecorder::start(Answer::Challenge);
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--no-recovery-key",
            "--remote",
            &recorder.url("me/vault.git"),
        ],
        &unlocks().with_value(""),
    );
    assert_eq!(r.code, Code::LockedOrAuth);
    let r = env.run(&["push"]);
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    for want in [HOST, "gage auth login"] {
        assert!(r.stderr.contains(want), "{want}: {}", r.stderr);
    }
}

// Go: TestPushNamesTheConflictingPathsJustLikeSync
#[test]
fn push_names_the_conflicting_paths_just_like_sync() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("shared.txt", "their version", "theirs");
    local_commit(&vault, "shared.txt", "my version");

    let r = env.run(&["push"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("shared.txt"), "{}", r.stderr);
}

// Go: TestSyncRefusesOverUncommittedChanges. Merging over them would
// publish half-typed work in an automatic merge commit.
#[test]
fn sync_refuses_over_uncommitted_changes() {
    let env = Env::new();
    let (vault, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("theirs.txt", "theirs", "theirs");
    local_commit(&vault, "mine.txt", "mine");
    std::fs::write(vault.join("scratch.txt"), "half-typed").unwrap();

    let r = env.run(&["sync"]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("scratch.txt"), "{}", r.stderr);
    assert!(!Device::clone(remote.path()).exists("scratch.txt"));
}

// Go: TestSyncLineSaysAlreadyInSyncWhenThereWasNothingToDo, end to end as
// Go's comment asks once F2 is fixed (Q-SYNC-PUSHED), and on the line
// itself.
#[test]
fn sync_line_says_already_in_sync_when_there_was_nothing_to_do() {
    let env = Env::new();
    let (path, _remote) = env.init_with_remote("personal");
    let vault = env.vault("personal");
    let line = sync_line(&vault, &SyncReport::default());
    assert!(line.contains("already in sync with origin"), "{line}");
    assert!(line.contains("personal"), "{line}");
    assert_eq!(gitrepo::ahead_count(&path).unwrap(), 0);

    for verb in ["sync", "push", "pull"] {
        let r = env.run(&[verb]);
        assert_eq!(r.code, Code::Success, "{verb}: {}", r.stderr);
        assert!(
            r.stdout.contains("already in sync with origin"),
            "{verb}: {}",
            r.stdout
        );
    }
}

// Go: TestSyncReportsAPushEvenWhenNothingWasSent, ported with its
// assertion flipped (Q-SYNC-PUSHED): Go pinned a known bug, F2, and said
// fixing it would turn the test red. A push returns whether anything was
// sent, so an in-sync `sync` claims no push.
#[test]
fn sync_reports_no_push_when_nothing_was_sent() {
    let env = Env::new();
    let (_vault, _remote) = env.init_with_remote("personal");
    let r = env.run(&["sync"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(!r.stdout.contains("pushed"), "{}", r.stdout);
}

// Go: TestReportConflictsSaysRecipientFilesAreNeverMerged
#[test]
fn report_conflicts_says_recipient_files_are_never_merged() {
    let env = Env::new();
    let (path, _remote) = env.init_with_remote("personal");
    let vault = env.vault("personal");
    let rendered = |conflicts: &[&str]| {
        let report = SyncReport {
            diverged: true,
            conflicts: conflicts.iter().map(|c| c.to_string()).collect(),
            ..SyncReport::default()
        };
        report_conflicts(&vault, &report).join("\n")
    };

    let recipients = rendered(&[".age-recipients"]);
    assert!(recipients.contains(".age-recipients"), "{recipients}");
    assert!(
        recipients.contains("never merge them for you"),
        "{recipients}"
    );
    assert!(recipients.contains(path.to_str().unwrap()), "{recipients}");
    // What to do next, since `gage sync` refuses these too (M10's settling
    // of P4's follow-up).
    assert!(recipients.contains("reconcile"), "{recipients}");
    assert!(recipients.contains("gage recipient verify"), "{recipients}");
    assert!(!recipients.contains("run `gage sync`"), "{recipients}");

    let entry = rendered(&["entries/11111111-1111-4111-8111-111111111111.age"]);
    assert!(!entry.contains("never merge them for you"), "{entry}");

    assert_eq!(rendered(&[]), "");
}

// In a session the verbs open the vault without unlocking it, and a pull
// through one invalidates the session's index.
#[test]
fn sync_verbs_in_a_session_need_no_unlock() {
    let env = Env::new();
    let (_, remote) = env.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("notes.txt", "x", "theirs");
    // No passphrase lines: an unlock would eat the next command as one.
    let r = env.session_ok(&["pull", "push -u personal", "sync", "exit"]);
    assert_eq!(
        passphrase_prompts(&r),
        0,
        "a sync verb unlocked:\n{}",
        r.stderr
    );
    assert!(r.stdout.contains("caught"), "{}", r.stdout);
    assert!(
        r.stdout.contains("already in sync with origin"),
        "{}",
        r.stdout
    );
}

// --- conflict resolution ----------------------------------------------------

/// A vault genuinely diverged on one entry, both versions real ciphertext
/// this device can read (Go's `conflictingVault`). The other device's
/// version of Alpha is a copy of Beta's file: a real entry for this
/// vault's recipients, now at Alpha's path. Returns the vault's path,
/// Alpha's id, and the remote, which lives as long as the test holds it.
fn conflicting_vault(env: &Env) -> (PathBuf, uuid::Uuid, BareRemote) {
    let (path, remote) = env.init_with_remote("personal");
    let alpha = env.insert_ok("alpha-secret", &["Alpha"]);
    let beta = env.insert_ok("beta-secret", &["Beta"]);

    let vault = env.vault("personal");
    let identity = vault.unlock(&unlocks()).unwrap();
    let other = Device::clone(remote.path());
    let beta_file = std::fs::read(path.join(format!("entries/{beta}.age"))).unwrap();
    other.write_bytes(&format!("entries/{alpha}.age"), &beta_file);
    other.commit("the other device's edit");
    other.push();

    // This device edits the same entry; its push finds the remote moved.
    let mut e = vault.read_entry(alpha, &identity).unwrap();
    e.value = "alpha-rotated-locally".into();
    vault.update(alpha, &e, &FakePrompter::new()).unwrap();
    (path, alpha, remote)
}

/// Where HEAD is, as far as gage's own git surface can say: how many
/// commits it has and the last one's message. A merge moves both.
fn head(dir: &Path) -> (usize, String) {
    (
        gitrepo::commit_count(dir).unwrap(),
        gitrepo::head_commit(dir).unwrap().message,
    )
}

// Go: TestSyncResolvesAConflictInteractivelyAndPushes. The milestone's
// definition of done through the CLI: two devices edited one entry, a
// human answered, and the result reached the remote.
#[test]
fn sync_resolves_a_conflict_interactively_and_pushes() {
    let env = Env::new();
    let (path, alpha, _remote) = conflicting_vault(&env);
    let p = unlocks().with_resolutions(&[Resolution::KeepBoth]);
    let r = env.run_with(&["sync"], &p);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(p.conflicts_presented().len(), 1);
    assert_eq!(p.conflicts_presented()[0].id, alpha);
    assert!(
        r.stdout
            .contains("resolved 1 entry in \"personal\", merged origin's changes and pushed"),
        "{}",
        r.stdout
    );
    // Keep both leaves three entries where there were two.
    assert_eq!(entry_files(&path).len(), 3);
    // And the merge reached the remote: the next push has nothing to do.
    let r = env.run(&["push"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("already in sync"), "{}", r.stdout);
}

// The same through a session and the real terminal menu: the key `ls`
// unlocked is not asked for again, and the answer is read from the
// session's own input.
#[test]
fn sync_in_a_session_resolves_with_the_held_key() {
    let env = Env::new();
    let (path, alpha, _remote) = conflicting_vault(&env);
    // The menu's own answer is read as a line, not a command: `r` alone
    // would be an ambiguous command prefix.
    let r = env.session_ok(&["ls", PASSPHRASE, "sync", "r", "exit"]);
    assert_eq!(
        passphrase_prompts(&r),
        1,
        "resolution re-prompted:\n{}",
        r.stderr
    );
    assert!(
        r.stderr.contains("Entry conflicts: \"Alpha\""),
        "{}",
        r.stderr
    );
    assert!(r.stdout.contains("resolved 1 entry"), "{}", r.stdout);
    assert_eq!(entry_files(&path).len(), 2);
    // Keep remote: Alpha now holds the other device's version, which was
    // a copy of Beta's.
    let entries = env.entries("personal");
    let (_, e) = entries.iter().find(|(id, _)| *id == alpha).unwrap();
    assert_eq!(e.value, "beta-secret");
}

// Go: TestSyncWithoutATerminalRefusesRatherThanChoosing. The real
// terminal prompter, with no terminal: a script or CI.
#[test]
fn sync_without_a_terminal_refuses_rather_than_choosing() {
    let env = Env::new();
    let (path, _, _remote) = conflicting_vault(&env);
    let before = head(&path);
    let out = SharedOut::default();
    let p = TerminalPrompter::new(
        std::io::Cursor::new(format!("{PASSPHRASE}\nb\n").into_bytes()),
        out.clone(),
        SecretInput::Lines,
    );
    let r = env.run_with(&["sync"], &p);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("needs someone to ask"), "{}", r.stderr);
    assert!(!out.text().contains("[l/r/b/s/q]"), "{}", out.text());
    assert_eq!(head(&path), before, "HEAD moved with nobody to ask");
    assert_eq!(entry_files(&path).len(), 2);
}

// Go: TestYesDoesNotResolveConflicts. Since M10, `--yes` is a real root
// flag: the run is accepted, reaches the conflict, and must refuse rather
// than answer it. `--yes` answers the recipient-change question only, and
// passes the conflict to the prompter underneath, which here has nobody
// to ask. A `--yes` that answered conflicts itself would move HEAD.
#[test]
fn yes_does_not_resolve_conflicts() {
    let env = Env::new();
    let (path, alpha, _remote) = conflicting_vault(&env);
    let before = head(&path);
    let p = unlocks();
    let r = env.run_with(&["sync", "--yes"], &p);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(!r.stderr.contains("unexpected argument"), "{}", r.stderr);
    assert!(r.stderr.contains("needs someone to ask"), "{}", r.stderr);
    assert!(r.stderr.contains(&alpha.to_string()), "{}", r.stderr);
    assert_eq!(
        p.conflicts_presented().len(),
        1,
        "never reached the conflict"
    );
    assert_eq!(head(&path), before, "`sync --yes` resolved a conflict");
}

// Skipping ends with exit Conflict, the conflict named, nothing pushed.
#[test]
fn sync_with_a_skip_exits_conflict_and_pushes_nothing() {
    let env = Env::new();
    let (path, alpha, _remote) = conflicting_vault(&env);
    let before = head(&path);
    let p = unlocks().with_resolutions(&[Resolution::Skip]);
    let r = env.run_with(&["sync"], &p);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("1 of 1"), "{}", r.stderr);
    assert!(r.stderr.contains(&alpha.to_string()), "{}", r.stderr);
    assert_eq!(head(&path), before);
}

// Go: TestPluralEntries (sync_cli_test.go), moved from M8a2: the
// "resolved N entries" line is its only caller. "1 entry", not "1 entries".
#[test]
fn plural_entries() {
    for (n, want) in [(0, "0 entries"), (1, "1 entry"), (2, "2 entries")] {
        assert_eq!(entry_count(n), want);
    }
    let env = Env::new();
    env.init_with_remote("personal");
    let vault = env.vault("personal");
    let line = |resolved| {
        sync_line(
            &vault,
            &SyncReport {
                merged: true,
                pushed: true,
                resolved,
                ..SyncReport::default()
            },
        )
    };
    assert!(line(1).contains("resolved 1 entry in"), "{}", line(1));
    assert!(line(3).contains("resolved 3 entries in"), "{}", line(3));
    assert!(
        line(0).contains("merged origin's changes into"),
        "{}",
        line(0)
    );
}

/// A prompt stream a test can read after handing it to the prompter.
#[derive(Clone, Default)]
struct SharedOut(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl SharedOut {
    fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

impl std::io::Write for SharedOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// --- clone ------------------------------------------------------------------

/// A vault initialized and seeded on one machine, published to a bare
/// remote; then a second, empty machine to clone from it (Go's
/// `publishedVault`). The first machine is returned so its files outlive
/// the test body.
fn published_vault() -> (Env, BareRemote, Env) {
    let first = Env::new();
    let (_, remote) = first.init_with_remote("personal");
    first.insert_ok("hunter2", &["ProtonMail"]);
    (first, remote, Env::new())
}

impl Env {
    fn clone_ok(&self, args: &[&str]) -> CliResult {
        let mut full = vec!["clone"];
        full.extend_from_slice(args);
        let r = self.run_with(&full, &FakePrompter::new());
        assert_eq!(r.code, Code::Success, "{full:?}: {}", r.stderr);
        r
    }
}

// Go: TestCloneProducesARegisteredWorkingVault
#[test]
fn clone_produces_a_registered_working_vault() {
    let (_first, remote, env) = published_vault();
    env.clone_ok(&[url(&remote), "--name", "personal"]);

    let entry = env.entry("personal");
    assert_eq!(entry.vault_type, "git");
    assert_eq!(entry.git.origin, url(&remote));
    assert_eq!(entry.device, crate::entry_tests::DEVICE);
    assert!(!entry.id.is_empty());
    assert!(entry.pubkey.is_empty(), "a clone holds no identity");
    for rel in [
        ".gage/config.toml",
        ".age-recipients",
        "entries",
        ".gitattributes",
    ] {
        assert!(entry.path.join(rel).exists(), "{rel}");
    }
    assert_eq!(entry_files(&entry.path).len(), 1);
    // A registration every other command accepts.
    Vault::from_registration(&env.dirs, "personal", &entry).unwrap();
}

// Go: TestCloneInfersTheNameFromTheURLAndNameOverridesIt
#[test]
fn clone_infers_the_name_from_the_url_and_name_overrides_it() {
    let (_first, remote, env) = published_vault();
    env.clone_ok(&[url(&remote)]);
    // gittest's bare remotes are named remote.git.
    assert!(env.global().vaults.contains_key("remote"));

    let (_first, remote, env) = published_vault();
    env.clone_ok(&[url(&remote), "--name", "work"]);
    assert!(env.global().vaults.contains_key("work"));
    assert!(!env.global().vaults.contains_key("remote"));
}

// Go: TestCloneWithoutDirLandsUnderGageData
#[test]
fn clone_without_dir_lands_under_gage_data() {
    let (_first, remote, env) = published_vault();
    env.clone_ok(&[url(&remote), "--name", "personal"]);
    assert_eq!(
        env.entry("personal").path,
        paths::vaults_dir(&env.dirs).join("personal")
    );
    // And --dir is honored.
    let (_first, remote, env) = published_vault();
    let dir = env.root.path().join("elsewhere");
    env.clone_ok(&[url(&remote), "--name", "p", "--dir", dir.to_str().unwrap()]);
    assert_eq!(env.entry("p").path, dir);
    assert!(dir.join(".age-recipients").exists());
}

// Go: TestCloneSaysSoWhenThisDeviceCannotDecryptAnything. Cloning does not
// grant access, and says so plainly instead of leaving a vault that
// silently decrypts nothing.
#[test]
fn clone_says_so_when_this_device_cannot_decrypt_anything() {
    let (_first, remote, env) = published_vault();
    let r = env.clone_ok(&[url(&remote), "--name", "personal", "--device", "phone"]);
    assert!(r.stdout.contains("cannot decrypt"), "{}", r.stdout);
    assert!(r.stdout.contains("gage identity add"), "{}", r.stdout);
    assert!(r.stdout.contains("\"phone\""), "{}", r.stdout);
    assert_eq!(env.entry("personal").device, "phone");
}

// Go: TestCloneRefusesAnUnrecognizedFormatVersion. And beyond Go's row: a
// refused clone leaves no registration and no half-made directory.
#[test]
fn clone_refuses_an_unrecognized_format_version() {
    let (_first, remote, env) = published_vault();
    let other = Device::clone(remote.path());
    let raw = other.read(".gage/config.toml");
    let current = format!("format_version = {}", vaultconfig::CURRENT_FORMAT_VERSION);
    assert!(raw.contains(&current), "{raw}");
    other.write_commit_push(
        ".gage/config.toml",
        &raw.replace(&current, "format_version = 99"),
        "a newer gage wrote this vault",
    );

    let r = env.run_with(
        &["clone", url(&remote), "--name", "personal"],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        !paths::global_config_file(&env.dirs).exists()
            || !env.global().vaults.contains_key("personal")
    );
    assert!(
        !paths::vaults_dir(&env.dirs).join("personal").exists(),
        "the refused clone left its directory behind"
    );
}

// Beyond Go's rows: a clone that fails in git leaves nothing either, and
// one into a --dir that already existed (empty) leaves it existing and
// empty rather than deleting it.
#[test]
fn a_refused_clone_leaves_no_half_made_vault() {
    let env = Env::new();
    let missing = env.root.path().join("no-such-remote.git");
    let r = env.run_with(
        &["clone", missing.to_str().unwrap(), "--name", "x"],
        &FakePrompter::new(),
    );
    assert_ne!(r.code, Code::Success);
    assert!(!paths::vaults_dir(&env.dirs).join("x").exists());
    assert!(!paths::global_config_file(&env.dirs).exists() || env.global().vaults.is_empty());

    let dir = env.root.path().join("mine");
    std::fs::create_dir(&dir).unwrap();
    let r = env.run_with(
        &[
            "clone",
            missing.to_str().unwrap(),
            "--name",
            "x",
            "--dir",
            dir.to_str().unwrap(),
        ],
        &FakePrompter::new(),
    );
    assert_ne!(r.code, Code::Success);
    assert!(dir.is_dir(), "the user's own directory was removed");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    // A --dir with someone's files in it is never touched, even though
    // the clone into it fails.
    let (_first, remote, env) = published_vault();
    let full = env.root.path().join("full");
    std::fs::create_dir(&full).unwrap();
    std::fs::write(full.join("keep.txt"), "mine").unwrap();
    let r = env.run_with(
        &[
            "clone",
            url(&remote),
            "--name",
            "x",
            "--dir",
            full.to_str().unwrap(),
        ],
        &FakePrompter::new(),
    );
    assert_ne!(r.code, Code::Success);
    assert_eq!(
        std::fs::read_to_string(full.join("keep.txt")).unwrap(),
        "mine"
    );
    assert_eq!(std::fs::read_dir(&full).unwrap().count(), 1);
}

// Through the real terminal prompter, the warnings reach stderr: being
// offline costs one warning on the unlock's pull and one on the write's
// push, and the command still succeeds.
#[test]
fn offline_warnings_reach_the_terminal() {
    let env = Env::new();
    let r = env.run_with(
        &[
            "init",
            "personal",
            "--remote",
            &gittest::refused_url(),
            "--no-recovery-key",
        ],
        &unlocks().with_value(""),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let r = env.session_ok(&["insert x", crate::lifecycle_tests::PASSPHRASE, "v", "exit"]);
    let text = r.stderr;
    assert_eq!(
        text.matches("continuing with the local copy").count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("committed locally but not pushed: could not reach origin")
            .count(),
        1,
        "{text}"
    );
}

// Go: TestCloneRefusesAnAlreadyRegisteredName
#[test]
fn clone_refuses_an_already_registered_name() {
    let (_first, remote, env) = published_vault();
    env.clone_ok(&[url(&remote), "--name", "personal"]);
    let r = env.run_with(
        &["clone", url(&remote), "--name", "personal"],
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(r.stderr.contains("--name"), "{}", r.stderr);
}

// Go: TestCloneIsOneShotOnly
#[test]
fn clone_is_one_shot_only() {
    let clone = registry()
        .find("clone")
        .cloned()
        .expect("clone is registered");
    assert!(!clone.availability.session_visible());
    assert!(clone.availability.one_shot_visible());
}

// The name check runs on what clone was given, before anything is
// fetched: a traversal never becomes a directory.
#[test]
fn clone_refuses_a_name_that_escapes_the_vaults_directory() {
    let (_first, remote, env) = published_vault();
    for bad in ["..", "../evil", "a/b"] {
        let r = env.run_with(
            &["clone", url(&remote), "--name", bad],
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Usage, "{bad}: {}", r.stderr);
    }
    assert!(!paths::vaults_dir(&env.dirs).exists());
}

// Go: TestCurrentFormatVersionIsWhatCloneChecks
#[test]
fn current_format_version_is_what_clone_checks() {
    assert_ne!(vaultconfig::CURRENT_FORMAT_VERSION, 99);
}

// --- parity -----------------------------------------------------------------

// The help-fixture check: each command's description, long help and flags
// as Go's help shows them.
#[test]
fn p4_sync_commands_have_their_go_fixture_flags_and_descriptions() {
    let env = Env::new();
    for (name, fixture, flags) in [
        (
            "clone",
            include_str!("../../../tests/fixtures/go/help/clone.txt"),
            &["--device", "--dir", "--name"][..],
        ),
        (
            "sync",
            include_str!("../../../tests/fixtures/go/help/sync.txt"),
            &["-u", "--use"][..],
        ),
        (
            "pull",
            include_str!("../../../tests/fixtures/go/help/pull.txt"),
            &["-u", "--use"][..],
        ),
        (
            "push",
            include_str!("../../../tests/fixtures/go/help/push.txt"),
            &["-u", "--use"][..],
        ),
    ] {
        let help = env.ok(&[name, "--help"]).stdout;
        let first = fixture.lines().next().unwrap().trim_end_matches('.');
        assert!(help.starts_with(first), "{name}:\n{help}");
        // Every prose line of the long help, and every flag's text.
        for line in fixture
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("Usage:"))
        {
            assert!(help.contains(line.trim()), "{name}: {line:?}\n{help}");
        }
        for line in fixture.lines().filter(|l| l.trim_start().starts_with('-')) {
            let text = line.split("   ").last().unwrap().trim();
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
        for flag in flags {
            assert!(help.contains(flag), "{name}: {flag}\n{help}");
        }
    }
    let init = env.ok(&["init", "--help"]).stdout;
    assert!(init.contains("--remote"), "{init}");
    assert!(
        init.contains("git remote (origin) URL; omit to start local-only"),
        "{init}"
    );
    assert!(
        init.contains("publishes the new vault immediately"),
        "{init}"
    );

    let root = env.ok(&["help"]).stdout;
    for short in [
        "Clone an existing vault from a remote",
        "Pull, merge what can be merged, then push",
        "Fast-forward this vault from its remote",
        "Publish this vault's commits to its remote",
    ] {
        assert!(root.contains(short), "{short}\n{root}");
    }
    let reg = registry();
    for verb in ["sync", "pull", "push"] {
        let c = reg.find(verb).unwrap();
        assert_eq!(c.group, crate::registry::GROUP_SYNC);
        assert!(c.availability.session_visible() && c.availability.one_shot_visible());
    }
    assert_eq!(
        reg.find("clone").unwrap().group,
        crate::registry::GROUP_VAULT
    );
}

// Go: TestNoCommandDoublesGagesOwnPrefix, for the sync verbs and the
// warnings a write's push or an unlock's pull add (ledger `ddce8b1`). A
// warning reaches the prompter, so each case's warnings are checked with
// its output; the real prompter prints them as they are.
#[test]
fn no_sync_failure_or_warning_doubles_gages_own_prefix() {
    use crate::error::ERROR_PREFIX;
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    // (case, output, warnings)
    let mut results: Vec<(&str, CliResult, Vec<String>)> = Vec::new();
    let mut run = |name, env: &Env, args: &[&str], p: FakePrompter| {
        let r = env.run_with(args, &p);
        results.push((name, r, p.warnings()));
    };

    let offline = Env::new();
    let refused = gittest::refused_url();
    run(
        "init offline",
        &offline,
        &[
            "init",
            "personal",
            "--remote",
            &refused,
            "--no-recovery-key",
        ],
        unlocks().with_value(""),
    );
    run("push offline", &offline, &["push"], FakePrompter::new());
    run(
        "insert offline",
        &offline,
        &["insert", "x"],
        with_value("v"),
    );
    run("show offline", &offline, &["show", "x"], unlocks());

    let diverged = Env::new();
    let (vault, remote) = diverged.init_with_remote("personal");
    Device::clone(remote.path()).write_commit_push("shared.txt", "theirs", "theirs");
    local_commit(&vault, "shared.txt", "mine");
    run("ls diverged", &diverged, &["ls"], unlocks());
    run("pull diverged", &diverged, &["pull"], FakePrompter::new());
    run("push diverged", &diverged, &["push"], FakePrompter::new());
    run("sync diverged", &diverged, &["sync"], FakePrompter::new());
    run(
        "insert diverged",
        &diverged,
        &["insert", "y"],
        with_value("v"),
    );
    std::fs::write(vault.join("scratch.txt"), "x").unwrap();
    run("sync dirty", &diverged, &["sync"], FakePrompter::new());
    run(
        "sync unregistered",
        &diverged,
        &["sync", "--use", "ghost"],
        FakePrompter::new(),
    );
    run(
        "clone nothing",
        &Env::new(),
        &["clone", ""],
        FakePrompter::new(),
    );

    let mut said = 0;
    for (name, r, warnings) in &results {
        let lines = r
            .stdout
            .lines()
            .chain(r.stderr.lines())
            .chain(warnings.iter().map(String::as_str));
        for line in lines {
            assert!(!line.contains(&doubled), "{name}: {line}");
            assert!(line.matches(ERROR_PREFIX).count() <= 1, "{name}: {line}");
        }
        for w in warnings {
            assert!(w.starts_with(ERROR_PREFIX), "{name}: {w}");
        }
        if !r.stderr.is_empty() {
            assert!(r.stderr.starts_with(ERROR_PREFIX), "{name}:\n{}", r.stderr);
        }
        if !r.stderr.is_empty() || !warnings.is_empty() {
            said += 1;
        }
    }
    // Every case above has something to say; one that went quiet has
    // stopped exercising what it is here for.
    let quiet: Vec<&str> = results
        .iter()
        .filter(|(_, r, w)| r.stderr.is_empty() && w.is_empty())
        .map(|(n, _, _)| *n)
        .collect();
    assert!(quiet.is_empty(), "quiet cases: {quiet:?}");
    assert_eq!(said, results.len());
}
