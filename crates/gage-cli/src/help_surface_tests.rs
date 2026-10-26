//! The whole command surface against Go's help fixtures: every fixture has
//! its registered command, and every one-shot command has a fixture. The
//! per-phase `*_have_their_go_fixture_flags_and_descriptions` tests stay:
//! they also check descriptions and long help, which this does not.
//! A reference for name, aliases, group, arguments and flags, not a byte
//! gate (Q-HELP-RENDER).

use std::collections::BTreeSet;

use clap::Command;

use crate::cli;
use crate::fixture_support::{
    all_fixtures, fixture_aliases, fixture_children, fixture_flag_specs, fixture_path,
    fixture_positionals, root_listing,
};
use crate::registry::{Registry, registry};
use crate::version::BuildInfo;

/// Go's help fixtures, captured at `GO_GAGE_SHA`: the root, six parents and
/// forty commands. A directory that went missing fails on this count
/// instead of passing for lack of anything to check.
const FIXTURE_COUNT: usize = 47;

/// The flags in those fixtures' `Flags:` blocks (Cobra's `-h/--help`
/// excluded). Pinned so a parser that stops finding flags, after a
/// recapture or an edit, fails here instead of checking nothing.
const FLAG_COUNT: usize = 78;

/// Registered one-shot commands, parents and aliases Go does not have. Each
/// entry names the `Q-` entry that records the deviation. Starts empty: a
/// Rust-only command is a decision, not a drive-by.
const RUST_ONLY: &[(&str, &str)] = &[];

fn clap_tree(reg: &Registry) -> Command {
    cli::build(reg, &BuildInfo::stamped())
}

fn clap_command<'a>(root: &'a Command, path: &[String]) -> Option<&'a Command> {
    path.iter()
        .try_fold(root, |cmd, name| cmd.find_subcommand(name))
}

fn rust_only(path: &str) -> bool {
    RUST_ONLY.iter().any(|(name, _)| *name == path)
}

#[test]
fn every_go_help_fixture_has_its_registered_command() {
    let fixtures = all_fixtures();
    assert_eq!(
        fixtures.len(),
        FIXTURE_COUNT,
        "fixtures: {:?}",
        fixtures.iter().map(|(s, _)| s).collect::<Vec<_>>()
    );
    let reg = registry();
    let tree = clap_tree(&reg);
    let root_text = &fixtures.iter().find(|(s, _)| s == "root").unwrap().1;
    let listing = root_listing(root_text);
    // 40 listed commands and nothing else: the listing is read, not skipped.
    assert_eq!(listing.len(), 40, "{listing:?}");

    let flags: usize = fixtures
        .iter()
        .map(|(_, text)| fixture_flag_specs(text).len())
        .sum();
    assert_eq!(flags, FLAG_COUNT, "the Flags: blocks were not all parsed");

    let mut problems = Vec::new();
    let mut leaves = 0;
    let mut parents = 0;
    for (stem, text) in &fixtures {
        let path = fixture_path(stem);
        let joined = path.join(" ");
        if path.is_empty() {
            continue;
        }
        let Some(cmd) = clap_command(&tree, &path) else {
            problems.push(format!("{joined}: no such command in the clap tree"));
            continue;
        };
        if let Some(info) = reg.commands.iter().find(|c| c.name == joined) {
            leaves += 1;
            // Aliases: Cobra lists the command's own name among them.
            let name = path.last().unwrap();
            let mut want: Vec<String> = fixture_aliases(text)
                .into_iter()
                .filter(|a| a != name)
                .collect();
            want.sort();
            let mut have: Vec<String> = info.aliases.iter().map(|a| a.to_string()).collect();
            have.sort();
            if want != have {
                problems.push(format!("{joined}: aliases {have:?}, Go has {want:?}"));
            }
            let mut clap_aliases: Vec<String> = cmd.get_all_aliases().map(str::to_string).collect();
            clap_aliases.sort();
            if clap_aliases != have {
                problems.push(format!("{joined}: clap aliases {clap_aliases:?}"));
            }
            // Group: the heading `root.txt` lists it under.
            let listed = listing
                .iter()
                .find(|(_, names)| names[0] == joined)
                .map(|(group, _)| group.as_str());
            if listed != Some(info.group) {
                problems.push(format!("{joined}: group {}, Go has {listed:?}", info.group));
            }
            // Positionals, by name, optionality and variadicness.
            let want: Vec<_> = fixture_positionals(text);
            let have: Vec<_> = cmd
                .get_positionals()
                .map(|a| {
                    (
                        a.get_id().as_str().to_lowercase(),
                        a.is_required_set(),
                        a.get_num_args().is_some_and(|n| n.max_values() > 1),
                    )
                })
                .collect();
            let want: Vec<_> = want
                .into_iter()
                .map(|p| (p.name, p.required, p.many))
                .collect();
            if want != have {
                problems.push(format!("{joined}: positionals {have:?}, Go has {want:?}"));
            }
            // Flags: each spelling Go lists exists on the clap command.
            for flag in fixture_flag_specs(text) {
                let found = cmd.get_arguments().any(|a| {
                    flag.long.as_deref().is_none_or(|l| a.get_long() == Some(l))
                        && flag.short.is_none_or(|c| a.get_short() == Some(c))
                        && a.get_action().takes_values() == flag.takes_value
                        && !a.is_hide_set()
                });
                if !found {
                    problems.push(format!("{joined}: missing flag {flag:?}"));
                }
            }
        } else if let Some(parent) = reg.parent(&joined) {
            parents += 1;
            // Every child Go lists is registered, and the reverse.
            let want: BTreeSet<String> = fixture_children(text).into_iter().collect();
            let have: BTreeSet<String> = reg
                .one_shot()
                .iter()
                .filter_map(|c| c.name.strip_prefix(&format!("{} ", parent.name)))
                .map(str::to_string)
                .collect();
            if want != have {
                problems.push(format!("{joined}: children {have:?}, Go has {want:?}"));
            }
            let clap_children: BTreeSet<String> = cmd
                .get_subcommands()
                .filter(|c| !c.is_hide_set())
                .map(|c| c.get_name().to_string())
                .collect();
            if want != clap_children {
                problems.push(format!("{joined}: clap children {clap_children:?}"));
            }
        } else {
            problems.push(format!("{joined}: not in the registry"));
        }
    }
    assert_eq!((leaves, parents), (40, 6), "{problems:#?}");
    assert!(problems.is_empty(), "{problems:#?}");

    // The root listing is exactly the registered one-shot commands, with
    // their aliases beside them.
    let mut listed: Vec<Vec<String>> = listing.iter().map(|(_, n)| n.clone()).collect();
    let mut registered: Vec<Vec<String>> = reg
        .one_shot()
        .iter()
        .map(|c| {
            std::iter::once(c.name)
                .chain(c.aliases.iter().copied())
                .map(str::to_string)
                .collect()
        })
        .collect();
    listed.sort();
    registered.sort();
    assert_eq!(listed, registered);
}

#[test]
fn every_one_shot_command_has_a_go_help_fixture() {
    let fixtures = all_fixtures();
    assert_eq!(fixtures.len(), FIXTURE_COUNT);
    let stems: BTreeSet<String> = fixtures
        .iter()
        .map(|(s, _)| fixture_path(s).join(" "))
        .collect();
    let reg = registry();

    let mut missing = Vec::new();
    for info in reg.one_shot() {
        if !stems.contains(info.name) && !rust_only(info.name) {
            missing.push(info.name.to_string());
        }
    }
    for parent in &reg.parents {
        if !stems.contains(parent.name) && !rust_only(parent.name) {
            missing.push(parent.name.to_string());
        }
    }
    // The clap tree is what a user can actually type: a visible command or
    // alias there that the registry does not account for is Rust-only too.
    fn walk(cmd: &Command, prefix: &str, fixtures: &BTreeSet<String>, out: &mut Vec<String>) {
        for sub in cmd.get_subcommands().filter(|c| !c.is_hide_set()) {
            let path = format!("{prefix}{}", sub.get_name());
            if !fixtures.contains(&path) && !rust_only(&path) {
                out.push(path.clone());
            }
            for alias in sub.get_all_aliases() {
                let alias = format!("{prefix}{alias}");
                if !fixtures.contains(&alias) && !rust_only(&alias) {
                    // An alias has no fixture of its own: it is covered
                    // when the command's `Aliases:` line lists it.
                    let canonical = fixtures.contains(&path);
                    let listed = canonical && alias_in_fixture(&path, &alias);
                    if !listed {
                        out.push(format!("{alias} (alias of {path})"));
                    }
                }
            }
            walk(sub, &format!("{path} "), fixtures, out);
        }
    }
    fn alias_in_fixture(path: &str, alias: &str) -> bool {
        let stem = path.replace(' ', "_");
        let name = alias.rsplit(' ').next().unwrap();
        all_fixtures()
            .iter()
            .find(|(s, _)| *s == stem)
            .is_some_and(|(_, text)| fixture_aliases(text).iter().any(|a| a == name))
    }
    walk(&clap_tree(&reg), "", &stems, &mut missing);
    assert!(
        missing.is_empty(),
        "Rust-only commands need a Q- entry and a RUST_ONLY line: {missing:#?}"
    );
}
