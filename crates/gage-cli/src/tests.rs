//! In-process CLI tests: `run_with` against injected IO and a fake
//! prompter. Help tests assert structure (grouping, aliases, the one-shot
//! set equal to the registry), not Cobra's bytes (Q-HELP-RENDER), and use
//! test registries where M0b's real one has no such commands yet.

use std::collections::BTreeSet;

use gage::prompter::FakePrompter;

use super::*;
use crate::cli::command_list_line;
use crate::error::ERROR_PREFIX;
use crate::registry::{
    Availability, CommandInfo, GROUP_ENTRY, GROUP_VAULT, ParentInfo, Registry, registry,
};

struct CliResult {
    code: Code,
    stdout: String,
    stderr: String,
}

fn run_reg(reg: &Registry, args: &[&str], stdin_is_terminal: bool) -> CliResult {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let mut io = Io {
        stdin: &mut std::io::empty(),
        stdout: &mut stdout,
        stderr: &mut stderr,
        stdin_is_terminal,
        build: version::resolve_build_info(Some("v1.2.3"), Some("abcdef1")),
    };
    let code = run_with(reg, &args, &mut io, &FakePrompter::new());
    CliResult {
        code,
        stdout: String::from_utf8(stdout).unwrap(),
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn run(args: &[&str]) -> CliResult {
    run_reg(&registry(), args, false)
}

fn cmd(
    name: &'static str,
    aliases: &'static [&'static str],
    group: &'static str,
    availability: Availability,
) -> CommandInfo {
    CommandInfo {
        name,
        aliases,
        short: "injected for this test only",
        group,
        usage: "",
        availability,
    }
}

/// The real registry plus a `vault` parent with three children, the
/// shape M1's real `vault`/`git` parents take.
fn registry_with_parent() -> Registry {
    let mut reg = registry();
    reg.parents.push(ParentInfo {
        name: "vault",
        short: "Manage registered vaults",
    });
    for (name, short) in [
        ("vault list", "List registered vaults"),
        ("vault info", "Show a vault's type, method and remote"),
        ("vault remove", "Forget a vault locally"),
    ] {
        reg.commands.push(CommandInfo {
            short,
            ..cmd(name, &[], GROUP_VAULT, Availability::Both)
        });
    }
    reg
}

/// Every command line in the root listing: indented lines other than the
/// usage line.
fn listed_lines(help: &str) -> BTreeSet<String> {
    help.lines()
        .filter(|l| l.starts_with("  ") && l.trim() != "gage [command]")
        .map(|l| l[2..].to_string())
        .collect()
}

/// Every runnable command path in a clap tree: leaves, since a parent only
/// groups. The walk recurses: a depth-1 walk would see only `vault` and
/// pass while every leaf under it went unregistered.
fn command_tree_paths(root: &clap::Command) -> Vec<String> {
    fn walk(c: &clap::Command, prefix: &mut Vec<String>, out: &mut Vec<String>) {
        for sub in c.get_subcommands() {
            prefix.push(sub.get_name().to_string());
            if sub.has_subcommands() {
                walk(sub, prefix, out);
            } else {
                out.push(prefix.join(" "));
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

// --- registry -------------------------------------------------------------

// Go: TestRegistryCompleteness. Every clap command is registered and every
// registry entry has a clap command, so a command added outside the
// registry fails here at once.
#[test]
fn registry_completeness() {
    for reg in [registry(), registry_with_parent()] {
        let tree = cli::build(&reg, &BuildInfo::stamped());
        let paths: BTreeSet<String> = command_tree_paths(&tree).into_iter().collect();
        let registered: BTreeSet<String> =
            reg.commands.iter().map(|c| c.name.to_string()).collect();
        assert_eq!(paths, registered);
    }
}

// Go: TestRegistryCompletenessDetectsNestedDrift. Proves the walk above
// would catch an unregistered nested command.
#[test]
fn registry_completeness_detects_nested_drift() {
    let tree = cli::build(&registry(), &BuildInfo::stamped())
        .subcommand(clap::Command::new("zzz-fake-group").subcommand(clap::Command::new("leaf")));
    let paths = command_tree_paths(&tree);
    assert!(
        paths.contains(&"zzz-fake-group leaf".to_string()),
        "{paths:?}"
    );
    assert!(
        !paths.contains(&"zzz-fake-group".to_string()),
        "a group is not runnable"
    );
    assert_eq!(registry().find("zzz-fake-group leaf"), None);
}

// Go: TestAliasResolvesToSameCommandAtCLILevel
#[test]
fn alias_resolves_to_same_command_at_cli_level() {
    for (canonical, alias) in [("status", "whoami"), ("exit", "quit")] {
        let (a, b) = (run(&[canonical]), run(&[alias]));
        assert_eq!(a.code, b.code);
        assert_eq!(a.stderr, b.stderr, "{canonical} vs {alias}");
    }
}

// --- help -----------------------------------------------------------------

// Go: TestHelpFlagExitsZero
#[test]
fn help_flag_exits_zero() {
    let r = run(&["--help"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("Usage:\n  gage [command]"),
        "{}",
        r.stdout
    );
    assert_eq!(r.stderr, "");
}

// Go: TestHelpCommandExitsZeroAndMatchesHelpFlag
#[test]
fn help_command_exits_zero_and_matches_help_flag() {
    for reg in [registry(), registry_with_parent()] {
        let (flag, command) = (
            run_reg(&reg, &["--help"], false),
            run_reg(&reg, &["help"], false),
        );
        assert_eq!(command.code, Code::Success);
        assert_eq!(command.stdout, flag.stdout);
        assert_eq!(run_reg(&reg, &["-h"], false).stdout, flag.stdout);
    }
}

// Go: TestHelpSubcommandPrintsUsageAndExitsZero
#[test]
fn help_subcommand_prints_usage_and_exits_zero() {
    let r = run(&["help", "use"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("Usage:\n  gage use <vault>"),
        "{}",
        r.stdout
    );
    assert_eq!(run(&["use", "--help"]).stdout, r.stdout);
}

// Go: TestHelpUnknownTopicFailsWithUsageCode
#[test]
fn help_unknown_topic_fails_with_usage_code() {
    for topic in [
        &["help", "not-a-real-command"][..],
        &["help", "use", "extra"],
    ] {
        let r = run(topic);
        assert_eq!(r.code, Code::Usage, "{topic:?}");
        assert!(r.stderr.contains("unknown help topic"), "{}", r.stderr);
        assert_eq!(r.stdout, "");
    }
}

// Go: TestHelpOutputIsGrouped
#[test]
fn help_output_is_grouped() {
    let mut reg = registry();
    reg.commands
        .push(cmd("fake-entry-cmd", &[], GROUP_ENTRY, Availability::Both));
    reg.commands
        .push(cmd("fake-vault-cmd", &[], GROUP_VAULT, Availability::Both));

    let out = run_reg(&reg, &["--help"], false).stdout;

    let vault = out.find("Vault lifecycle:\n").expect("vault header");
    let entry = out.find("Entry CRUD:\n").expect("entry header");
    assert!(
        vault < entry,
        "registry group order, not registry order:\n{out}"
    );
    assert!(out[vault..entry].contains("fake-vault-cmd"));
    assert!(out[entry..].contains("fake-entry-cmd"));
}

// Go: TestOneShotHelpMatchesRegistryExactly. Read from the registry, not
// by parsing names out of prose: the listed lines are exactly the
// one-shot commands' lines. The injected entry keeps the positive half
// from passing vacuously on M0b's all-session registry.
#[test]
fn one_shot_help_matches_registry_exactly() {
    let mut reg = registry_with_parent();
    reg.commands.push(cmd(
        "fake-oneshot",
        &["fake-alias"],
        GROUP_VAULT,
        Availability::OneShotOnly,
    ));

    let out = run_reg(&reg, &["--help"], false).stdout;

    let want: BTreeSet<String> = reg.one_shot().iter().map(command_list_line).collect();
    assert!(
        want.iter()
            .any(|l| l.starts_with("fake-oneshot, fake-alias"))
    );
    assert_eq!(listed_lines(&out), want);
}

// Go: TestNeitherHelpSpellingListsSessionOnlyCommands
#[test]
fn neither_help_spelling_lists_session_only_commands() {
    for args in [&["--help"][..], &["help"], &[]] {
        let out = run(args).stdout;
        let listed = listed_lines(&out);
        for c in registry()
            .commands
            .iter()
            .filter(|c| c.availability == Availability::SessionOnly)
        {
            for name in std::iter::once(&c.name).chain(c.aliases) {
                assert!(
                    !listed
                        .iter()
                        .any(|l| l.split([' ', ',']).next() == Some(*name)),
                    "{args:?} lists session-only {name}:\n{out}"
                );
            }
        }
    }
}

// Go: TestAliasesListedAlongsideCanonicalName
#[test]
fn aliases_listed_alongside_canonical_name() {
    let mut reg = registry();
    reg.commands
        .push(cmd("search", &["grep"], GROUP_ENTRY, Availability::Both));
    for c in reg.commands.iter().filter(|c| !c.aliases.is_empty()) {
        let line = command_list_line(c);
        let name_field = line.split("  ").next().unwrap();
        let mut want = vec![c.usage_name()];
        want.extend(c.aliases.iter().map(|a| a.to_string()));
        assert_eq!(name_field, want.join(", "), "{line:?}");
    }
    let out = run_reg(&reg, &["--help"], false).stdout;
    assert!(out.contains("  search, grep  "), "{out}");
}

// Go: TestCommandHelpListsAliasesSortedWithTheCanonicalName
#[test]
fn command_help_lists_aliases_sorted_with_the_canonical_name() {
    let r = run(&["help", "status"]);
    assert!(
        r.stdout.contains("Aliases:\n  status, whoami\n"),
        "{}",
        r.stdout
    );
    // Sorted, so the canonical name isn't always first.
    let r = run(&["help", "exit"]);
    assert!(
        r.stdout.contains("Aliases:\n  exit, quit\n"),
        "{}",
        r.stdout
    );
    let mut reg = registry();
    reg.commands
        .push(cmd("search", &["grep"], GROUP_ENTRY, Availability::Both));
    let r = run_reg(&reg, &["help", "search"], false);
    assert!(
        r.stdout.contains("Aliases:\n  grep, search\n"),
        "{}",
        r.stdout
    );

    assert!(!run(&["help", "use"]).stdout.contains("Aliases:"));
}

// Go: TestParentCommandHelpListsItsChildren (M1 adds the same assertion for
// the real `vault` and `git` parents).
#[test]
fn parent_command_help_lists_its_children() {
    let r = run_reg(&registry_with_parent(), &["vault"], false);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("Manage registered vaults"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("Available Commands:"), "{}", r.stdout);
    for child in ["list", "info", "remove"] {
        assert!(
            r.stdout.contains(&format!("  {child} ")),
            "missing {child}:\n{}",
            r.stdout
        );
    }
}

// Go: TestParentCommandChildrenListedWithTheirShortText
#[test]
fn parent_command_children_listed_with_their_short_text() {
    let reg = registry_with_parent();
    let out = run_reg(&reg, &["vault"], false).stdout;
    let short = reg.find("vault info").unwrap().short;
    assert!(
        out.lines()
            .any(|l| l.trim_start().starts_with("info") && l.ends_with(short)),
        "{out}"
    );
}

// Go: TestParentCommandHelpMatchesAcrossBareDashHelpAndHelpCommand
#[test]
fn parent_command_help_matches_across_bare_dash_help_and_help_command() {
    let reg = registry_with_parent();
    let bare = run_reg(&reg, &["vault"], false).stdout;
    assert!(bare.contains("Available Commands:"));
    assert_eq!(run_reg(&reg, &["vault", "--help"], false).stdout, bare);
    assert_eq!(run_reg(&reg, &["help", "vault"], false).stdout, bare);
}

// Go: TestLeafCommandHelpUnchangedByTheChildrenListing
#[test]
fn leaf_command_help_unchanged_by_the_children_listing() {
    let reg = registry_with_parent();
    let r = run_reg(&reg, &["vault", "list", "--help"], false);
    assert_eq!(r.code, Code::Success);
    assert!(
        r.stdout.contains("Usage:\n  gage vault list"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("Available Commands:"), "{}", r.stdout);
    assert_eq!(
        run_reg(&reg, &["help", "vault", "list"], false).stdout,
        r.stdout
    );
}

// Go's TestListsCommandEntryIgnoresProse is excused: it tests a helper that
// parsed names out of Cobra's help text. These tests read the registry and
// clap's command tree instead.

// --- usage errors ---------------------------------------------------------

// Go: TestWrongArgCountShowsCommandUsage
#[test]
fn wrong_arg_count_shows_command_usage() {
    let r = run(&["use"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(
        r.stderr.contains("required arguments were not provided"),
        "{}",
        r.stderr
    );
    assert!(
        r.stderr.contains("Usage:\n  gage use <vault>"),
        "{}",
        r.stderr
    );
    assert_eq!(r.stdout, "", "usage help goes to stderr, never stdout");
}

// Go: TestUnknownFlagShowsCommandUsage
#[test]
fn unknown_flag_shows_command_usage() {
    let r = run(&["status", "--no-such-flag"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("--no-such-flag"), "{}", r.stderr);
    assert!(r.stderr.contains("Usage:\n  gage status"), "{}", r.stderr);
    assert_eq!(r.stdout, "");
}

// Go: TestUnknownCommandShowsFullHelpListing
#[test]
fn unknown_command_shows_full_help_listing() {
    let r = run(&["frobnicate"]);
    assert_eq!(r.code, Code::Usage);
    assert!(r.stderr.contains("frobnicate"), "{}", r.stderr);
    assert!(
        r.stderr.contains(r#"Use "gage help <command>""#),
        "{}",
        r.stderr
    );
    // Unknown under a parent shows that parent's help.
    let r = run_reg(&registry_with_parent(), &["vault", "nope"], false);
    assert_eq!(r.code, Code::Usage);
    assert!(r.stderr.contains("Available Commands:"), "{}", r.stderr);
}

// Go: TestCodedApplicationErrorsDoNotGrowUsageText. Only parse rejections
// get usage appended; an error a command chose keeps its own message.
// (Go's TestSessionWrongArgCountShowsCommandUsage moves to P3 · M6: there
// is no session yet.)
#[test]
fn coded_application_errors_do_not_grow_usage_text() {
    for args in [&["status"][..], &["help", "nope"]] {
        let r = run(args);
        assert_ne!(r.code, Code::Success);
        assert!(!r.stderr.contains("Usage:"), "{args:?}:\n{}", r.stderr);
    }
}

// --- error prefix ---------------------------------------------------------

// Go: TestNoCommandDoublesGagesOwnPrefix, over the failures that exist in
// M0b. Process rule: every milestone that adds a command adds its failing
// cases to this table.
#[test]
fn no_command_doubles_gages_own_prefix() {
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    let cases: &[(&str, &[&str], bool)] = &[
        ("unknown command", &["no-such-command"], false),
        ("unknown flag", &["status", "--no-such-flag"], false),
        ("wrong argument count", &["use"], false),
        ("session-only command in one-shot mode", &["status"], false),
        ("unknown help topic", &["help", "nope"], false),
        ("bare gage on a terminal", &[], true),
    ];
    for (name, args, tty) in cases {
        let r = run_reg(&registry(), args, *tty);
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

// --- version and dispatch -------------------------------------------------

// Go: TestVersionFlagPrintsVersionAndCommit
#[test]
fn version_flag_prints_version_and_commit() {
    let r = run(&["--version"]);
    assert_eq!(r.code, Code::Success);
    assert_eq!(r.stdout, "gage v1.2.3 (commit abcdef1)\n");
}

// Bare `gage` on a terminal goes to the session entry point, which fails
// until P3 · M6.
#[test]
fn bare_gage_on_a_terminal_dispatches_to_the_session() {
    let r = run_reg(&registry(), &[], true);
    assert_eq!(r.code, Code::Internal);
    assert_eq!(r.stderr, "gage: session mode is not implemented yet\n");
    assert_eq!(r.stdout, "");
}

// Bare `gage` with stdin redirected prints the root help, since --stdin is
// the explicit spelling for reading session commands from stdin.
#[test]
fn bare_gage_with_redirected_stdin_prints_root_help() {
    let r = run_reg(&registry(), &[], false);
    assert_eq!(r.code, Code::Success);
    assert_eq!(r.stdout, run(&["--help"]).stdout);
}

// A stub that isn't session-only reports it is not implemented, as Go's
// stubs do, rather than looking like an unknown command.
#[test]
fn an_unimplemented_one_shot_command_is_internal() {
    let r = run_reg(&registry_with_parent(), &["vault", "list"], false);
    assert_eq!(r.code, Code::Internal);
    assert_eq!(r.stderr, "gage: vault list is not implemented yet\n");
}
