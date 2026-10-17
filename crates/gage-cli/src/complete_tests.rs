//! Tab completion and command abbreviation, below the terminal: the
//! candidate functions are plain and typed, so everything about *which*
//! candidates are right at a position is tested here, in-process. Ports
//! Go's `complete_test.go`; each test names its Go row. The one adapter
//! test is the last one here; the real-terminal one is
//! `tests/complete_pty.rs`.
//!
//! The `vault` parent keeps its own coverage beside the `identity` form
//! (M13's substitution, undone in P5 · M9a).

use std::sync::Arc;

use clap::{Arg, Command};

use gage::exitcode::Code;
use gage::locker::MemLocker;
use gage::prompter::FakePrompter;
use gage::session::{Session, SessionConfig};

use crate::BuildInfo;
use crate::cli;
use crate::complete::{
    completion_candidates, completion_words, entry_title_candidates, flag_candidates,
    leaf_positional_candidates, prefix_matches, resolve_session_command_name,
    subcommand_candidates, top_level_session_names, vault_name_candidates,
};
use crate::error::CliError;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::registry::{Registry, registry};
use crate::repl::exec_line;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn root(reg: &Registry) -> Command {
    cli::build(reg, &BuildInfo::stamped())
}

/// A session over `env`'s registered vaults, built the way `run_session`
/// builds one (as Go's `newSessionApp` does): global config's current
/// vault is current from the start, named but still locked.
fn session<'a>(env: &'a Env, prompter: &'a FakePrompter) -> Session<'a> {
    let dirs = env.dirs.clone();
    Session::new(SessionConfig {
        open: Box::new(move |name| {
            crate::commands::open_registered(&dirs, name)
                .map(|v| v.with_locker(Arc::new(MemLocker)))
        }),
        prompter,
        idle_timeout: None,
        current: Some(env.global().current),
        now: None,
    })
}

/// A session with `personal` current and unlocked, holding the titles
/// `AWS` and `ProtonMail`: anything that wrongly offers titles or vault
/// names at a position has some to offer.
fn unlocked_personal<'a>(env: &'a Env, prompter: &'a FakePrompter) -> Session<'a> {
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "hunter2");
    env.add("personal", "AWS", "secret");
    let mut s = session(env, prompter);
    s.use_vault("personal").unwrap();
    s
}

fn one_passphrase() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

/// A prompter that fails the test the instant it's asked anything:
/// completion must never turn into a prompt.
fn panics_if_asked() -> FakePrompter {
    FakePrompter::new()
}

/// What `completion_candidates` offers at the end of `line`.
fn complete_at(
    reg: &Registry,
    env: &Env,
    session: Option<&mut Session>,
    line: &str,
) -> Vec<String> {
    let (words, partial) = completion_words(line);
    completion_candidates(reg, &root(reg), &env.dirs, session, &words, &partial)
}

// --- prefix_matches --------------------------------------------------------------

// Go: TestPrefixMatchesReturnsEveryPrefixHit
#[test]
fn prefix_matches_returns_every_prefix_hit() {
    let got = prefix_matches(&strs(&["lock", "log", "ls", "use"]), "l");
    assert_eq!(got, ["lock", "log", "ls"]);
}

// Go: TestPrefixMatchesRejectsSubstring. Pins "prefix only" against
// drifting toward the title resolver's substring rule.
#[test]
fn prefix_matches_rejects_substring() {
    assert!(prefix_matches(&strs(&["identity"]), "enti").is_empty());
}

// Go: TestPrefixMatchesDedupesAndSorts
#[test]
fn prefix_matches_dedupes_and_sorts() {
    assert_eq!(
        prefix_matches(&strs(&["use", "use", "lock"]), ""),
        ["lock", "use"]
    );
}

// --- top_level_session_names -------------------------------------------------------

// Go: TestTopLevelSessionNamesIncludesGroupsAndAliases.
#[test]
fn top_level_session_names_includes_groups_and_aliases() {
    let names = top_level_session_names(&registry());
    for want in [
        "vault",
        "git",
        "identity",
        "recipient",
        "use",
        "status",
        "whoami",
        "search",
        "grep",
        "quit",
    ] {
        assert!(names.iter().any(|n| n == want), "missing {want}: {names:?}");
    }
}

// Go: TestTopLevelSessionNamesExcludesInitAndClone (Q-CMD-AVAILABILITY).
#[test]
fn top_level_session_names_excludes_init_and_clone() {
    let names = top_level_session_names(&registry());
    for bad in ["init", "clone"] {
        assert!(!names.iter().any(|n| n == bad), "{bad} is one-shot only");
    }
}

// Go: TestCompletingPartialSessionCommandsMatchesExactly
#[test]
fn completing_partial_session_commands_matches_exactly() {
    let names = top_level_session_names(&registry());
    for (partial, want) in [("us", "use"), ("loc", "lock"), ("stat", "status")] {
        assert_eq!(prefix_matches(&names, partial), [want], "{partial}");
    }
}

// --- resolve_session_command_name --------------------------------------------------

fn resolved(word: &str) -> String {
    resolve_session_command_name(&registry(), word).unwrap()
}

// Go: TestResolveSessionCommandNameExactNameUnchanged
#[test]
fn resolve_session_command_name_exact_name_unchanged() {
    assert_eq!(resolved("use"), "use");
}

// Go: TestResolveSessionCommandNameExactAliasUnchanged
#[test]
fn resolve_session_command_name_exact_alias_unchanged() {
    assert_eq!(resolved("whoami"), "whoami");
}

// Go: TestResolveSessionCommandNameUnambiguousAbbreviation
#[test]
fn resolve_session_command_name_unambiguous_abbreviation() {
    assert_eq!(resolved("ident"), "identity");
    assert_eq!(resolved("vau"), "vault");
}

// Go: TestResolveSessionCommandNameAmbiguousAbbreviationReportsCandidates.
// Go's `l` matches lock, log and ls (`log` is M12a's).
#[test]
fn resolve_session_command_name_ambiguous_abbreviation_reports_candidates() {
    let err = resolve_session_command_name(&registry(), "l").unwrap_err();
    assert_eq!(err.candidates, ["lock", "log", "ls"]);
    let text = err.to_string();
    for want in ["lock", "log", "ls", "\"l\""] {
        assert!(text.contains(want), "{text}");
    }
}

// Go: TestResolveSessionCommandNameSubstringNotAbbreviation
#[test]
fn resolve_session_command_name_substring_not_abbreviation() {
    assert_eq!(resolved("enti"), "enti");
}

// Go: TestResolveSessionCommandNameUnknownWordUnchanged
#[test]
fn resolve_session_command_name_unknown_word_unchanged() {
    assert_eq!(resolved("nope"), "nope");
}

// Go: TestResolveSessionCommandNameNeverReachesInitOrClone. `ini` is a
// prefix of the one-shot-only `init`, and must not expand to it.
#[test]
fn resolve_session_command_name_never_reaches_init_or_clone() {
    for word in ["ini", "clo"] {
        assert_eq!(resolved(word), word);
    }
}

// Go: TestResolveSessionCommandNameOnAnEmptyWord. Every name is a prefix
// match for "", which would turn a stray empty token into
// ambiguous-everything instead of the ordinary unknown-command error.
#[test]
fn resolve_session_command_name_on_an_empty_word() {
    assert_eq!(resolved(""), "");
}

// --- completion_words ----------------------------------------------------------------

// Go: TestCompletionWordsOnNothingTyped
#[test]
fn completion_words_on_nothing_typed() {
    for head in ["", " ", "   ", "\t", " \t "] {
        assert_eq!(completion_words(head), (vec![], String::new()), "{head:?}");
    }
}

// Go: TestCompletionWordsMidWord
#[test]
fn completion_words_mid_word() {
    assert_eq!(completion_words("vau"), (vec![], "vau".to_string()));
}

// Go: TestCompletionWordsAfterASpace
#[test]
fn completion_words_after_a_space() {
    assert_eq!(
        completion_words("vault "),
        (strs(&["vault"]), String::new())
    );
}

// Go: TestCompletionWordsMultipleWordsPartialLast
#[test]
fn completion_words_multiple_words_partial_last() {
    assert_eq!(
        completion_words("show --use pers"),
        (strs(&["show", "--use"]), "pers".to_string())
    );
}

// --- flag_candidates / subcommand_candidates -----------------------------------------

// Go: TestFlagCandidatesMatchesTheRealCobraCommand, against the real
// clap command built from the registry.
#[test]
fn flag_candidates_matches_the_real_clap_command() {
    let reg = registry();
    let root = root(&reg);
    let insert = root.find_subcommand("insert").unwrap();
    let got = flag_candidates(insert);
    for want in [
        "--use",
        "-u",
        "--description",
        "--multiline",
        "-m",
        "--value-stdin",
        "--edit",
        "-e",
        "--force",
        "-f",
        "--field",
    ] {
        assert!(got.iter().any(|g| g == want), "missing {want}: {got:?}");
    }
    // Its own flags only: not another command's, and no positional.
    assert!(!got.iter().any(|g| g == "--to-vault"), "{got:?}");
    assert!(got.iter().all(|g| g.starts_with('-')), "{got:?}");
}

// Go: TestFlagCandidatesSkipsHiddenFlags. gage registers no hidden flag
// today, so this builds one: the rule has to hold the first time one is
// added.
#[test]
fn flag_candidates_skips_hidden_flags() {
    let mut cmd = Command::new("demo")
        .arg(Arg::new("shown").long("shown"))
        .arg(Arg::new("secret").long("secret-debug").hide(true));
    cmd.build();
    let got = flag_candidates(&cmd);
    assert!(got.contains(&"--shown".to_string()), "{got:?}");
    assert!(!got.contains(&"--secret-debug".to_string()), "{got:?}");
}

// Go: TestSubcommandCandidatesListsIdentityChildren
#[test]
fn subcommand_candidates_lists_identity_children() {
    let reg = registry();
    let root = root(&reg);
    let identity = root.find_subcommand("identity").unwrap();
    assert_eq!(
        sorted(subcommand_candidates(&reg, identity)),
        ["add", "enroll", "list"]
    );
}

// The same function on the `vault` parent M13 used before `identity`
// existed.
#[test]
fn subcommand_candidates_lists_the_parents_children() {
    let reg = registry();
    let root = root(&reg);
    let vault = root.find_subcommand("vault").unwrap();
    assert_eq!(
        sorted(subcommand_candidates(&reg, vault)),
        ["info", "list", "remove", "set-default"]
    );
}

// Go: TestSubcommandCandidatesSkipsHiddenChildren. The child has no
// registry entry, which counts as visible: only the hidden one is left
// out.
#[test]
fn subcommand_candidates_skips_hidden_children() {
    let parent = Command::new("demo")
        .subcommand(Command::new("visible").alias("v"))
        .subcommand(Command::new("concealed").hide(true));
    let got = subcommand_candidates(&registry(), &parent);
    assert!(got.contains(&"visible".to_string()), "{got:?}");
    assert!(got.contains(&"v".to_string()), "its alias too: {got:?}");
    assert!(!got.contains(&"concealed".to_string()), "{got:?}");
}

// --- vault and entry sources -----------------------------------------------------------

// Go: TestVaultNameCandidatesListsRegisteredVaults
#[test]
fn vault_name_candidates_lists_registered_vaults() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    assert_eq!(vault_name_candidates(&env.dirs), ["personal", "work"]);
}

// A config that can't be read has nowhere to report it: no candidates.
#[test]
fn vault_name_candidates_with_no_config_is_empty() {
    assert!(vault_name_candidates(&Env::new().dirs).is_empty());
}

// Go: TestEntryTitleCandidatesNoSessionIsNil
#[test]
fn entry_title_candidates_with_no_session_is_empty() {
    assert!(entry_title_candidates(None).is_empty());
}

// --- leaf_positional_candidates ----------------------------------------------------------

// Go: TestLeafPositionalCandidatesOffersNothingForACommandWithNoQuery.
// Only use/lock and the entry-query commands have a first positional
// worth completing; Go's `sync` joins the list in P4. Against a
// registered vault and an unlocked one with titles, so a command routed
// to either source by mistake would have something to offer.
#[test]
fn leaf_positional_candidates_offers_nothing_for_a_command_with_no_query() {
    let env = Env::new();
    let p = one_passphrase();
    let mut s = unlocked_personal(&env, &p);
    let reg = registry();
    let root = root(&reg);
    for name in ["ls", "init", "reindex", "status"] {
        let target = root.find_subcommand(name).unwrap();
        assert!(
            leaf_positional_candidates(target, &[], &env.dirs, Some(&mut s)).is_empty(),
            "{name}"
        );
    }
}

// Go: TestLeafPositionalCandidatesStopAfterTheFirstArgument. `rename
// <title> <new-title>` is why: completing the new title against existing
// ones would suggest the very collisions rename exists to avoid.
#[test]
fn leaf_positional_candidates_stop_after_the_first_argument() {
    let env = Env::new();
    let p = one_passphrase();
    let mut s = unlocked_personal(&env, &p);
    let reg = registry();
    let root = root(&reg);
    for name in ["rename", "use", "lock", "show"] {
        let target = root.find_subcommand(name).unwrap();
        assert!(
            !leaf_positional_candidates(target, &[], &env.dirs, Some(&mut s)).is_empty(),
            "{name} offers nothing even at its first argument"
        );
        let typed = strs(&["already-typed"]);
        assert!(
            leaf_positional_candidates(target, &typed, &env.dirs, Some(&mut s)).is_empty(),
            "{name}"
        );
    }
}

// --- completion_candidates ------------------------------------------------------------------

// Go: TestCompletingTopLevelListsSessionVisibleNames. The whole set, even
// with `us` typed: narrowing to the partial word is the caller's
// `prefix_matches`, not this function's.
#[test]
fn completing_top_level_lists_session_visible_names() {
    let (reg, env) = (registry(), Env::new());
    let got = complete_at(&reg, &env, None, "us");
    assert!(got.contains(&"status".to_string()), "{got:?}");
    assert!(got.contains(&"use".to_string()), "{got:?}");
    for bad in ["init", "clone"] {
        assert!(!got.contains(&bad.to_string()), "{bad}: {got:?}");
    }
}

// Go: TestCompletingABareDashAtTheTopLevelOffersRootFlags
#[test]
fn completing_a_bare_dash_at_the_top_level_offers_root_flags() {
    let (reg, env) = (registry(), Env::new());
    let got = complete_at(&reg, &env, None, "-");
    assert!(!got.is_empty());
    assert!(got.iter().all(|g| g.starts_with('-')), "{got:?}");
    assert_eq!(got, flag_candidates(&root(&reg)));
}

// Go: TestCompletingFlagPositionListsCommandFlags
#[test]
fn completing_flag_position_lists_command_flags() {
    let (reg, env) = (registry(), Env::new());
    let got = complete_at(&reg, &env, None, "insert --");
    assert!(got.contains(&"--force".to_string()), "{got:?}");
}

// Go: TestCompletingSubcommandListsIdentityChildren
#[test]
fn completing_subcommand_lists_identity_children() {
    let (reg, env) = (registry(), Env::new());
    assert_eq!(
        sorted(complete_at(&reg, &env, None, "identity ")),
        ["add", "enroll", "list"]
    );
}

#[test]
fn completing_subcommand_lists_the_parents_children() {
    let (reg, env) = (registry(), Env::new());
    assert_eq!(
        sorted(complete_at(&reg, &env, None, "vault ")),
        ["info", "list", "remove", "set-default"]
    );
}

// Go: TestCompletingUseArgumentListsVaultNames
#[test]
fn completing_use_argument_lists_vault_names() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.init("work", &[]);
    assert_eq!(complete_at(&reg, &env, None, "use "), ["personal", "work"]);
}

// Go: TestCompletingLockArgumentListsVaultNames
#[test]
fn completing_lock_argument_lists_vault_names() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.init("work", &[]);
    assert_eq!(complete_at(&reg, &env, None, "lock "), ["personal", "work"]);
}

// Go: TestCompletingUseFlagValueListsVaultNames
#[test]
fn completing_use_flag_value_lists_vault_names() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.init("work", &[]);
    assert_eq!(
        complete_at(&reg, &env, None, "show --use "),
        ["personal", "work"]
    );
}

// Go: TestCompletingUseArgumentIsIndependentOfUnlockState. The set comes
// from global config, never from which vaults are unlocked.
#[test]
fn completing_use_argument_is_independent_of_unlock_state() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.init("work", &[]);
    let p = one_passphrase();
    let mut s = session(&env, &p);
    s.use_vault("personal").unwrap();
    assert_eq!(
        complete_at(&reg, &env, Some(&mut s), "use "),
        ["personal", "work"]
    );
}

// Go: TestCompletingAnEntryQueryListsCurrentVaultTitles
#[test]
fn completing_an_entry_query_lists_current_vault_titles() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "hunter2");
    env.add("personal", "AWS", "secret");
    let p = one_passphrase();
    let mut s = session(&env, &p);
    s.use_vault("personal").unwrap();
    assert_eq!(
        complete_at(&reg, &env, Some(&mut s), "show "),
        ["AWS", "ProtonMail"]
    );
}

// Go: TestCompletingAnEntryQueryWhenLockedNeverUnlocks, with a prompter
// that answers nothing: an unlock would fail the test. `personal` is
// current (from global config) but locked, so this is the locked path,
// not the no-current-vault one.
#[test]
fn completing_an_entry_query_when_locked_never_unlocks() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.add("personal", "AWS", "secret");
    let p = panics_if_asked();
    let mut s = session(&env, &p);
    assert_eq!(s.current(), Some("personal"));
    let got = complete_at(&reg, &env, Some(&mut s), "show ");
    assert!(got.is_empty(), "{got:?}");
    assert!(p.unlock_requests().is_empty(), "completion asked to unlock");
}

// Go: TestCompletingQueryAfterAUseFlagStillOffersEntryTitles. `show --use
// work <Tab>` is still the query at position 0, and still the *current*
// vault's titles: personal's, not work's.
#[test]
fn completing_query_after_a_use_flag_still_offers_entry_titles() {
    let (reg, env) = (registry(), Env::new());
    env.init("personal", &[]);
    env.init("work", &[]);
    env.add("personal", "ProtonMail", "hunter2");
    env.add("work", "GitHub", "tok3n");
    let p = one_passphrase();
    let mut s = session(&env, &p);
    s.use_vault("personal").unwrap();
    assert_eq!(
        complete_at(&reg, &env, Some(&mut s), "show --use work "),
        ["ProtonMail"]
    );
}

// Not Go rows: the other spellings of `--use`. `-u` takes a vault name
// like `--use` does, and both step over their value, attached or not,
// when counting positionals.
#[test]
fn completing_the_use_flags_other_spellings() {
    let (reg, env) = (registry(), Env::new());
    let p = one_passphrase();
    let mut s = unlocked_personal(&env, &p);
    env.init("work", &[]);
    let at = |s: &mut Session, line| complete_at(&reg, &env, Some(s), line);
    assert_eq!(at(&mut s, "show -u "), ["personal", "work"]);
    for line in ["show -u work ", "show --use=work ", "show -u=work "] {
        assert_eq!(at(&mut s, line), ["AWS", "ProtonMail"], "{line:?}");
    }
    // An attached value is not followed by one: `AWS` is the first
    // positional, so `rename`'s second (the new title) offers nothing.
    assert!(at(&mut s, "rename --use=work AWS ").is_empty());
}

// Not a Go row: a first word that names no command has no position to
// complete, so nothing is offered rather than some other command's set.
#[test]
fn completing_after_an_unknown_command_offers_nothing() {
    let (reg, env) = (registry(), Env::new());
    let p = one_passphrase();
    let mut s = unlocked_personal(&env, &p);
    assert!(complete_at(&reg, &env, Some(&mut s), "nope ").is_empty());
}

// Not a Go row: the registry's session filter one level down. A child the
// registry marks one-shot-only is left out, as session help leaves it out.
#[test]
fn subcommand_candidates_skip_children_not_available_in_a_session() {
    let mut reg = registry();
    let info = reg
        .commands
        .iter_mut()
        .find(|c| c.name == "vault info")
        .unwrap();
    info.availability = crate::registry::Availability::OneShotOnly;
    let root = root(&reg);
    let vault = root.find_subcommand("vault").unwrap();
    assert_eq!(
        sorted(subcommand_candidates(&reg, vault)),
        ["list", "remove", "set-default"]
    );
}

// --- dispatch --------------------------------------------------------------------------------

fn run_line(env: &Env, s: &mut Session, line: &str) -> (Result<bool, CliError>, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let dirs = env.dirs.clone();
    let mut stdin: &[u8] = b"";
    let res = {
        let mut io = crate::Io {
            stdin: &mut stdin,
            stdout: &mut out,
            stderr: &mut err,
            stdin_is_terminal: true,
            line_editor: false,
            build: BuildInfo::stamped(),
            dirs: &move || Ok(dirs.clone()),
            hostname: &|| panic!("read the hostname"),
            editor: None,
            passphrase_env: None,
            now: &std::time::Instant::now,
            clipboard: &crate::clipboard::fake::unused(),
            clipboard_wait: &|_| {},
        };
        exec_line(&registry(), s, &mut io, line)
    };
    (res, String::from_utf8(out).unwrap())
}

// Go: TestRunSessionCommandDispatchesAnUnambiguousAbbreviation
#[test]
fn run_session_command_dispatches_an_unambiguous_abbreviation() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = panics_if_asked();
    let mut s = session(&env, &p);
    let (res, out) = run_line(&env, &mut s, "stat");
    assert!(!res.unwrap());
    assert!(
        out.contains("no vaults have been unlocked"),
        "`stat` didn't run `status`:\n{out}"
    );
}

// Not a Go row: `status` above is a session verb, handled before clap;
// this is the other path, where the expanded word goes on to clap with
// the rest of the line. `vau list` runs `vault list`.
#[test]
fn run_session_command_dispatches_an_abbreviation_through_clap() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = panics_if_asked();
    let mut s = session(&env, &p);
    let (res, out) = run_line(&env, &mut s, "vau list");
    assert!(!res.unwrap());
    assert!(out.contains("personal"), "`vau list` didn't list:\n{out}");
}

// Go: TestRunSessionCommandRejectsAnAmbiguousAbbreviation. Candidates
// reported, nothing run, and the session still usable.
#[test]
fn run_session_command_rejects_an_ambiguous_abbreviation() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = panics_if_asked();
    let mut s = session(&env, &p);
    let (res, out) = run_line(&env, &mut s, "l");
    let err = res.unwrap_err();
    assert_eq!(err.code(), Code::Usage);
    for want in ["lock", "ls"] {
        assert!(err.to_string().contains(want), "{err}");
    }
    assert_eq!(out, "", "an ambiguous abbreviation ran something");
    let (res, _) = run_line(&env, &mut s, "status");
    assert!(res.is_ok(), "the session didn't survive it");
}

// Go: TestAbbreviationDoesNotReachSubcommands. Only the first token is
// rewritten, so `vault inf` is not `vault info`: it falls to clap's own
// unknown-subcommand handling, never running `info`.
#[test]
fn abbreviation_does_not_reach_subcommands() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = panics_if_asked();
    let mut s = session(&env, &p);
    let (res, out) = run_line(&env, &mut s, "vault inf");
    assert_eq!(res.unwrap_err().code(), Code::Usage);
    assert_eq!(out, "", "`inf` ran something");
}

// --- the rustyline adapter ----------------------------------------------------------------

// Go: TestSessionCompleterDoWiresCandidatesIntoReadlinesContract. The one
// thin test of the adapter: `complete` returns where the partial word
// starts and the full candidates, across a single match, several, none
// (a substring that isn't a prefix), a later word, and a quoted title.
#[test]
fn session_completer_complete_wires_candidates_into_rustylines_contract() {
    use rustyline::Context;
    use rustyline::completion::Completer;
    use rustyline::history::DefaultHistory;

    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "secret");
    let p = one_passphrase();
    let mut s = session(&env, &p);
    s.use_vault("personal").unwrap();
    let reg = registry();
    let helper = crate::complete::SessionHelper::new(
        reg.clone(),
        root(&reg),
        env.dirs.clone(),
        std::rc::Rc::new(std::cell::RefCell::new(s)),
    );
    let history = DefaultHistory::new();
    let complete = |line: &str| {
        let (start, pairs) = helper
            .complete(line, line.len(), &Context::new(&history))
            .unwrap();
        let shown: Vec<(String, String)> = pairs
            .into_iter()
            .map(|p| (p.display, p.replacement))
            .collect();
        (start, shown)
    };
    let same = |names: &[&str]| -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| (n.to_string(), n.to_string()))
            .collect()
    };

    assert_eq!(complete("u"), (0, same(&["use"])), "a single match");
    assert_eq!(complete("l"), (0, same(&["lock", "log", "ls"])), "several");
    assert_eq!(complete("enti"), (0, vec![]), "a substring offers nothing");
    assert_eq!(
        complete("vault s"),
        (6, same(&["set-default"])),
        "a later word"
    );
    assert_eq!(
        complete("show AW"),
        (
            5,
            vec![("AWS root account".into(), "\"AWS root account\"".into())]
        ),
        "a title with spaces is quoted so it parses back as one argument"
    );
}
