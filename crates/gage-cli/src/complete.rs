//! Tab completion and command abbreviation for the REPL (and only the
//! REPL: not one-shot mode, and no shell-completion scripts).
//!
//! Both share one primitive, [`prefix_matches`], over names the registry
//! says a session can run. It is *prefix only*, deliberately not the
//! substring rule `gage::entry::resolve_query` applies to entry titles:
//! `enti` never matches `identity`. The two resolvers stay separate rather
//! than one being generalized over both.
//!
//! Everything that decides what could go at a position is a plain function
//! over the clap tree, the registry and (for titles) a session, callable
//! without a terminal. [`SessionHelper`] is the thin `rustyline` adapter.
//! Completion never unlocks anything and never prompts, and entry titles
//! come from the *current* vault alone. See "Session model" in the design
//! doc and Go's Q-HELP-SURFACES / Q-CMD-AVAILABILITY.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use clap::Command;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};

use gage::config;
use gage::exitcode::Code;
use gage::paths;
use gage::session::Session;
use gage::xdgpaths::Dirs;

use crate::error::CliError;
use crate::registry::Registry;

/// Commands whose first positional addresses an entry: `show`, `cat`,
/// `edit`, `rename`, `rm`, and `generate`, whose argument names a *new*
/// entry but for which existing titles are still worth seeing (to avoid a
/// duplicate). Go's `mv` and `cp` join when they are registered. One list,
/// so the decision lives in one place.
const ENTRY_QUERY_COMMANDS: [&str; 8] = [
    "show", "cat", "edit", "rename", "generate", "rm", "mv", "cp",
];

/// Every candidate that starts with `prefix`, deduplicated and sorted.
pub fn prefix_matches(candidates: &[String], prefix: &str) -> Vec<String> {
    candidates
        .iter()
        .filter(|c| c.starts_with(prefix))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every top-level command word a session accepts: each session-visible
/// registry name's first word (so `vault` for `vault list`, though
/// `vault` is no entry of its own) and every alias. `init` never appears
/// (Q-CMD-AVAILABILITY): the same filter session help follows, read off
/// the same registry.
pub fn top_level_session_names(reg: &Registry) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for info in reg.session() {
        let top = info.name.split(' ').next().unwrap_or(info.name);
        for name in std::iter::once(top).chain(info.aliases.iter().copied()) {
            if !out.iter().any(|seen| seen == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// What an abbreviation matched more than one command with: refuse to
/// guess, and name them.
#[derive(Debug, PartialEq, Eq)]
pub struct AmbiguousCommand {
    pub prefix: String,
    pub candidates: Vec<String>,
}

impl std::fmt::Display for AmbiguousCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "gage: {:?} matches more than one command: {}",
            self.prefix,
            self.candidates.join(", ")
        )
    }
}

impl std::error::Error for AmbiguousCommand {}

impl From<AmbiguousCommand> for CliError {
    fn from(e: AmbiguousCommand) -> CliError {
        CliError::new(Code::Usage, e)
    }
}

/// Expands a typed first token into the command it abbreviates: unchanged
/// if it already names a command (or is a group such as `vault`) or
/// matches nothing, so the ordinary unknown-command path reports it; the
/// sole prefix match if exactly one session-visible name starts with it;
/// [`AmbiguousCommand`] if several do. Called on the first token only,
/// never a subcommand or a flag.
pub fn resolve_session_command_name(
    reg: &Registry,
    word: &str,
) -> Result<String, AmbiguousCommand> {
    // Every name matches the empty prefix, which would turn a stray empty
    // token into ambiguous-everything instead of an unknown command.
    if word.is_empty() || reg.find(word).is_some() {
        return Ok(word.to_string());
    }
    let names = top_level_session_names(reg);
    if names.iter().any(|n| n == word) {
        return Ok(word.to_string());
    }
    let mut matches = prefix_matches(&names, word);
    match matches.len() {
        0 => Ok(word.to_string()),
        1 => Ok(matches.remove(0)),
        _ => Err(AmbiguousCommand {
            prefix: word.to_string(),
            candidates: matches,
        }),
    }
}

/// Splits what is typed before the cursor into whole words and the
/// partial word under it. A plain whitespace split, not the line
/// splitter's quote-aware one: completion only needs to know where in the
/// command the cursor sits, and a partial word that opens a quote is
/// matched by its literal text.
pub fn completion_words(head: &str) -> (Vec<String>, String) {
    let mut words: Vec<String> = head.split_whitespace().map(str::to_string).collect();
    if words.is_empty() || head.ends_with(char::is_whitespace) {
        return (words, String::new());
    }
    let partial = words.pop().expect("non-empty above");
    (words, partial)
}

/// `cmd`'s own flags, long and short forms, read off the real clap
/// command rather than a second list. Hidden ones are left out.
pub fn flag_candidates(cmd: &Command) -> Vec<String> {
    let mut out = Vec::new();
    for arg in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        if let Some(long) = arg.get_long() {
            out.push(format!("--{long}"));
        }
        if let Some(short) = arg.get_short() {
            out.push(format!("-{short}"));
        }
    }
    out
}

/// `parent`'s child commands and their aliases, read off the real clap
/// tree. Hidden children are left out, as are those the registry marks
/// not available in a session; a child with no registry entry counts as
/// visible.
pub fn subcommand_candidates(reg: &Registry, parent: &Command) -> Vec<String> {
    let mut out = Vec::new();
    for child in parent.get_subcommands().filter(|c| !c.is_hide_set()) {
        let path = format!("{} {}", parent.get_name(), child.get_name());
        if reg
            .find(&path)
            .is_some_and(|info| !info.availability.session_visible())
        {
            continue;
        }
        out.push(child.get_name().to_string());
        out.extend(child.get_all_aliases().map(str::to_string));
    }
    out
}

/// The flags whose value is a vault name. `-u/--use` is every entry
/// command's vault selector; `--to-vault` (`mv`/`cp`) joins when those
/// are registered.
fn takes_vault_name(word: &str) -> bool {
    matches!(word, "-u" | "--use")
}

/// Every vault registered in global config, sorted: the candidates for
/// `use`, `lock` and `--use`, independent of which are unlocked. A config
/// that can't be read offers nothing: completion has nowhere to report
/// an error.
pub fn vault_name_candidates(dirs: &Dirs) -> Vec<String> {
    config::load(&paths::global_config_file(dirs))
        .map(|g| g.vaults.into_keys().collect())
        .unwrap_or_default()
}

/// The current vault's titles, or nothing outside a session or when the
/// session has none to offer. See `Session::entry_title_candidates` for
/// the guarantee that this never unlocks.
pub fn entry_title_candidates(session: Option<&mut Session>) -> Vec<String> {
    session
        .map(Session::entry_title_candidates)
        .unwrap_or_default()
}

/// `rest` (the args typed after the command) without its flags and their
/// values, so a positional's index is right when a flag precedes it:
/// `show --use work <Tab>` is position 0, not 2. Which flags take a value
/// is read off `target`'s own arguments.
fn positional_args(target: &Command, rest: &[String]) -> Vec<String> {
    let takes_value = |flag: &str| {
        let arg = match flag.strip_prefix("--") {
            Some(long) => target.get_arguments().find(|a| a.get_long() == Some(long)),
            None => {
                let mut chars = flag.chars().skip(1);
                match (chars.next(), chars.next()) {
                    (Some(short), None) => target
                        .get_arguments()
                        .find(|a| a.get_short() == Some(short)),
                    _ => None,
                }
            }
        };
        arg.and_then(|a| a.get_num_args())
            .is_some_and(|n| n.takes_values())
    };
    let mut out = Vec::new();
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        if !word.starts_with('-') {
            out.push(word.clone());
        } else if !word.contains('=') && takes_value(word) {
            words.next();
        }
    }
    out
}

/// Candidates for a leaf command's positional argument, given the
/// positionals already typed: vault names for `use`/`lock`'s one
/// argument, titles for the first argument of an entry-query command,
/// nothing at any later position.
pub fn leaf_positional_candidates(
    target: &Command,
    typed: &[String],
    dirs: &Dirs,
    session: Option<&mut Session>,
) -> Vec<String> {
    if !typed.is_empty() {
        return Vec::new();
    }
    match target.get_name() {
        "use" | "lock" => vault_name_candidates(dirs),
        name if ENTRY_QUERY_COMMANDS.contains(&name) => entry_title_candidates(session),
        _ => Vec::new(),
    }
}

/// The command `words` reaches below `root`, and the words typed after
/// it: descending by subcommand name or alias, stepping over flags, until
/// a leaf (or the words run out at a parent). `None` for an unknown word
/// or when no command was reached.
fn find_target<'c, 'w>(
    root: &'c Command,
    words: &'w [String],
) -> Option<(&'c Command, &'w [String])> {
    let mut cmd = root;
    let mut i = 0;
    while cmd.has_subcommands() && i < words.len() {
        let word = &words[i];
        i += 1;
        if !word.starts_with('-') {
            cmd = cmd.find_subcommand(word)?;
        }
    }
    (!std::ptr::eq(cmd, root)).then_some((cmd, &words[i..]))
}

/// Everything that could be typed at the cursor, given the whole words
/// before it and the partial word under it (`partial` only decides
/// whether a flag is being typed; narrowing to it is the caller's
/// [`prefix_matches`]).
pub fn completion_candidates(
    reg: &Registry,
    root: &Command,
    dirs: &Dirs,
    session: Option<&mut Session>,
    words: &[String],
    partial: &str,
) -> Vec<String> {
    if words.is_empty() {
        return if partial.starts_with('-') {
            flag_candidates(root)
        } else {
            top_level_session_names(reg)
        };
    }
    let Some((target, rest)) = find_target(root, words) else {
        return Vec::new();
    };
    if partial.starts_with('-') {
        return flag_candidates(target);
    }
    if words.last().is_some_and(|w| takes_vault_name(w)) {
        return vault_name_candidates(dirs);
    }
    if rest.is_empty() && target.has_subcommands() {
        return subcommand_candidates(reg, target);
    }
    leaf_positional_candidates(target, &positional_args(target, rest), dirs, session)
}

/// `rustyline`'s view of all this: the registry, the clap tree (built
/// once; completion only reads it) and the session it is typing into,
/// shared with the REPL, which holds it mutably only while a command runs,
/// never while the editor is reading, which is when this is called.
pub struct SessionHelper<'a> {
    reg: Registry,
    root: Command,
    dirs: Dirs,
    session: Rc<RefCell<Session<'a>>>,
}

impl<'a> SessionHelper<'a> {
    pub fn new(
        reg: Registry,
        root: Command,
        dirs: Dirs,
        session: Rc<RefCell<Session<'a>>>,
    ) -> SessionHelper<'a> {
        SessionHelper {
            reg,
            root,
            dirs,
            session,
        }
    }

    /// Where the partial word under the cursor starts, and the candidates
    /// for it. `rustyline` does the common-prefix fill and the listing.
    pub fn candidates(&self, head: &str) -> (usize, Vec<Pair>) {
        let (words, partial) = completion_words(head);
        // Not borrowable means a command is mid-run, which can't be while
        // the editor reads; offering nothing is the safe answer anyway.
        let mut guard = self.session.try_borrow_mut().ok();
        let all = completion_candidates(
            &self.reg,
            &self.root,
            &self.dirs,
            guard.as_deref_mut(),
            &words,
            &partial,
        );
        let pairs = prefix_matches(&all, &partial)
            .into_iter()
            .map(|c| Pair {
                replacement: quote_if_needed(&c),
                display: c,
            })
            .collect();
        (head.len() - partial.len(), pairs)
    }
}

/// A candidate containing whitespace is quoted, so the completed word
/// parses back as one argument (`show "AWS root account"`).
fn quote_if_needed(candidate: &str) -> String {
    if candidate.contains(char::is_whitespace) && !candidate.contains('"') {
        format!("\"{candidate}\"")
    } else {
        candidate.to_string()
    }
}

impl Completer for SessionHelper<'_> {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        Ok(self.candidates(&line[..pos]))
    }
}

impl Hinter for SessionHelper<'_> {
    type Hint = String;
}

impl Highlighter for SessionHelper<'_> {}

impl Validator for SessionHelper<'_> {}

impl Helper for SessionHelper<'_> {}
