//! `gage mv` and `gage cp` through the real CLI, in-process: Go's
//! `cmd/gage/move_test.go`. Each test names its Go row.

use gage::exitcode::Code;
use gage::gitrepo;
use gage::prompter::FakePrompter;

use crate::entry_tests::unlocks;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::registry::{GROUP_ENTRY, registry};
use crate::repl_tests::passphrase_prompts;
use crate::trustcache_cli_tests::commit_routine_change;

fn two_vaults() -> Env {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("shared", &[]);
    env
}

impl Env {
    fn titles_in(&self, vault: &str) -> String {
        self.cmd_ok(&["ls", "--use", vault]).stdout
    }

    fn commits_in(&self, vault: &str) -> usize {
        self.commits(vault)
    }
}

// Go: TestMvThroughTheCLIMovesEntryBetweenVaults.
#[test]
fn mv_through_the_cli_moves_entry_between_vaults() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");

    let r = env.cmd_ok(&["mv", "GitHub", "--to-vault", "shared", "--use", "personal"]);

    assert!(r.stdout.contains("moved"), "{}", r.stdout);
    assert!(r.stdout.contains("shared"), "{}", r.stdout);
    let got = env.cmd_ok(&["show", "GitHub", "--use", "shared"]);
    assert!(got.stdout.contains("hunter2"), "{}", got.stdout);
    assert!(!env.titles_in("personal").contains("GitHub"));
    assert_eq!(
        env.cmd(&["show", "GitHub", "--use", "personal"]).code,
        Code::NotFound
    );
}

// Go: TestCpThroughTheCLILeavesOriginalAndAddsCopy.
#[test]
fn cp_through_the_cli_leaves_original_and_adds_copy() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");

    let r = env.cmd_ok(&["cp", "GitHub", "--to-vault", "shared", "--use", "personal"]);

    assert!(r.stdout.contains("copied"), "{}", r.stdout);
    for vault in ["personal", "shared"] {
        let got = env.cmd_ok(&["show", "GitHub", "--use", vault]);
        assert!(got.stdout.contains("hunter2"), "{vault}: {}", got.stdout);
    }
}

// Go: TestMvRequiresToVaultFlag. A usage error, and nothing unlocked.
#[test]
fn mv_requires_the_to_vault_flag() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");
    let p = FakePrompter::new();

    for verb in ["mv", "cp"] {
        let r = env.run_with(&[verb, "GitHub", "--use", "personal"], &p);
        assert_eq!(r.code, Code::Usage, "{verb}: {}", r.stderr);
        assert!(r.stderr.contains("--to-vault"), "{verb}: {}", r.stderr);
    }
    assert!(p.unlock_requests().is_empty());
}

// Go: TestMvToUnregisteredVaultFailsCleanly. Nothing is decrypted: the
// source was never unlocked, and nothing was committed anywhere.
#[test]
fn mv_to_an_unregistered_vault_fails_cleanly() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");
    let before = (env.commits_in("personal"), env.commits_in("shared"));
    let p = FakePrompter::new();

    let r = env.run_with(
        &["mv", "GitHub", "--to-vault", "ghost", "--use", "personal"],
        &p,
    );

    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    assert!(r.stderr.contains("ghost"), "{}", r.stderr);
    assert!(p.unlock_requests().is_empty(), "the source was unlocked");
    assert_eq!(
        (env.commits_in("personal"), env.commits_in("shared")),
        before
    );
    assert!(env.titles_in("personal").contains("GitHub"));
}

// Go: TestMvToItsOwnSourceVaultIsRejected.
#[test]
fn mv_to_its_own_source_vault_is_rejected() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");
    let before = env.commits_in("personal");
    let p = FakePrompter::new();

    for verb in ["mv", "cp"] {
        let r = env.run_with(
            &[
                verb,
                "GitHub",
                "--to-vault",
                "personal",
                "--use",
                "personal",
            ],
            &p,
        );
        assert_eq!(r.code, Code::Usage, "{verb}: {}", r.stderr);
    }
    assert!(p.unlock_requests().is_empty(), "unlocked before refusing");
    assert_eq!(env.commits_in("personal"), before);

    // One vault under two names is the same refusal, by id.
    let mut global = env.global();
    let entry = global.vaults["personal"].clone();
    global.vaults.insert("alias".into(), entry);
    env.save_global(&global);
    let r = env.run_with(
        &["mv", "GitHub", "--to-vault", "alias", "--use", "personal"],
        &p,
    );
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(p.unlock_requests().is_empty());
}

// Go: TestMvDecliningTheTrustCheckAbortsThroughTheCLI. The destination's
// recipient list changed since this device last looked: a no leaves both
// vaults exactly as they were.
#[test]
fn mv_declining_the_trust_check_aborts_through_the_cli() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");
    env.cmd_ok(&["ls", "--use", "shared"]); // a reviewed baseline
    let shared = env.entry("shared").path;
    commit_routine_change(
        &shared,
        "phone-1",
        &age::x25519::Identity::generate().to_public().to_string(),
    );
    let before = (env.commits_in("personal"), env.commits_in("shared"));

    let p = unlocks().with_recipient_answers(&[false]);
    let r = env.run_with(
        &["mv", "GitHub", "--to-vault", "shared", "--use", "personal"],
        &p,
    );

    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert_eq!(p.recipient_changes().len(), 1);
    assert_eq!(p.recipient_changes()[0].vault, "shared");
    assert_eq!(
        (env.commits_in("personal"), env.commits_in("shared")),
        before
    );
    assert!(env.titles_in("personal").contains("GitHub"));
    assert!(gitrepo::is_clean(&shared).unwrap());
}

// Go: TestMvInASessionNeverUnlocksTheDestination. One passphrase, for the
// source; the entry arrives, and the session's own index follows.
#[test]
fn mv_in_a_session_never_unlocks_the_destination() {
    let env = two_vaults();
    env.add("personal", "GitHub", "hunter2");

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "ls",
        "mv GitHub --to-vault shared",
        "ls",
        "exit",
    ]);

    assert_eq!(passphrase_prompts(&r), 1, "{}{}", r.stdout, r.stderr);
    assert!(r.stdout.contains("moved"), "{}", r.stdout);
    // The second `ls` ran from the updated index: the entry is gone from it.
    let (first, second) = r.stdout.split_once("moved").unwrap();
    assert!(first.contains("GitHub"), "{first}");
    assert!(!second.lines().any(|l| l.starts_with("GitHub")), "{second}");
    let got = env.cmd_ok(&["show", "GitHub", "--use", "shared"]);
    assert!(got.stdout.contains("hunter2"), "{}", got.stdout);
}

// Not a Go row: P5's fixture-parity check for `mv` and `cp`, as the other
// P5 commands have: the Go help's description, flags and group.
#[test]
fn p5_mv_and_cp_have_their_go_fixture_flags_and_descriptions() {
    let env = Env::new();
    let reg = registry();
    for (name, fixture) in [
        ("mv", include_str!("../../../tests/fixtures/go/help/mv.txt")),
        ("cp", include_str!("../../../tests/fixtures/go/help/cp.txt")),
    ] {
        let help = env.ok(&[name, "--help"]).stdout;
        let first = fixture.lines().next().unwrap();
        assert!(help.starts_with(first), "{name}:\n{help}");
        for line in fixture.lines().filter(|l| l.trim_start().starts_with('-')) {
            let text = line.split("   ").last().unwrap().trim();
            assert!(help.contains(text), "{name}: {text:?}\n{help}");
        }
        for flag in ["--to-vault", "-u", "--use"] {
            assert!(help.contains(flag), "{name}: {flag}\n{help}");
        }
        let info = reg
            .find(name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        assert_eq!(info.short, fixture.lines().next().unwrap());
        assert_eq!(info.group, GROUP_ENTRY);
    }
    // In the entry group, after `rm`.
    let root = env.ok(&["--help"]).stdout;
    let at = |c: &str| {
        root.find(&format!("  {c} "))
            .unwrap_or_else(|| panic!("{c}\n{root}"))
    };
    assert!(at("rm") < at("mv") && at("mv") < at("cp"), "{root}");
}
