//! The clap command tree, built from the registry, and one-shot dispatch.
//!
//! clap parses and renders per-command help and usage errors
//! (ADR-0030). The root listing is gage's own: grouped in registry
//! group order, aliases beside the canonical name, one-shot commands only.
//! `gage --help` and `gage help` render it through the same function, so
//! they can't differ.

use clap::builder::ValueRange;
use clap::error::ErrorKind;
use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::exitcode::ExitCode;

use crate::error::CliError;
use crate::registry::{self, Availability, CommandInfo, Registry, SessionVerb};
use crate::version::BuildInfo;

const ROOT_ABOUT: &str = "gage — a git-backed, age-encrypted secret & notes manager";

/// The ids of the two script flags, as clap registers them on the root.
const SCRIPT_FLAG: &str = "script";
const STDIN_FLAG: &str = "stdin";

const ROOT_FOOTER: &str = r#"Use "gage help <command>" (or "gage <command> --help") for more information about a command."#;

/// Per-command help: description, usage, then children, arguments and
/// flags. An aliased command adds its alias line after the usage (see
/// [`leaf`]).
const COMMAND_TEMPLATE: &str = "{about-with-newline}\nUsage:\n  {usage}\n\n{all-args}";

/// What one-shot parsing resolved to.
#[derive(Debug)]
pub enum Outcome {
    /// Text for stdout, exit 0: help, version, a bare parent's help.
    Print(String),
    /// Bare `gage` on a terminal.
    Session,
    /// A registered leaf command with its matches.
    Command(registry::Command, ArgMatches),
    /// `--script FILE` or `--stdin` on bare `gage`: a whole session's
    /// commands from somewhere that isn't a prompt. It wins over the
    /// terminal check, so a script run from a terminal is still a script.
    Script(ScriptRun),
}

/// A non-interactive session to run: where its lines come from, and
/// whether `--yes` was given for the whole run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRun {
    pub source: ScriptSource,
    pub assume_yes: bool,
}

/// Where a non-interactive session's command lines come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSource {
    File(String),
    Stdin,
}

/// The full one-shot command tree for `reg`.
pub fn build(reg: &Registry, build: &BuildInfo) -> Command {
    let mut root = Command::new("gage")
        .about(ROOT_ABOUT)
        .version(build.version_text())
        // `-v, --version` in place of clap's `-V, --version`.
        .disable_version_flag(true)
        .arg(
            Arg::new("version")
                .short('v')
                .long("version")
                .action(ArgAction::Version),
        )
        // The first root persistent flag. Global, so it reaches every
        // encrypting command wherever it's typed; hidden, because help
        // shows no inherited flags. What it answers is one question: see
        // `assumeyes`.
        .arg(
            Arg::new(crate::assumeyes::FLAG)
                .long("yes")
                .action(ArgAction::SetTrue)
                .global(true)
                .hide(true)
                .help("answer the recipient-change confirmation with yes, without showing it (for scripts and CI)"),
        )
        // The other two root persistent flags: only ever meaningful on
        // bare `gage`, where they choose a whole session's input. Global
        // and hidden like `--yes`, so clap accepts them wherever typed and
        // `parse` can refuse the combinations that mean two things.
        .arg(
            Arg::new(SCRIPT_FLAG)
                .long("script")
                .value_name("FILE")
                .global(true)
                .hide(true)
                .help("run session commands from a file instead of a prompt"),
        )
        .arg(
            Arg::new(STDIN_FLAG)
                .long("stdin")
                .action(ArgAction::SetTrue)
                .global(true)
                .hide(true)
                .help("run session commands read from stdin instead of a prompt"),
        )
        .disable_help_subcommand(true)
        .override_help(render_root_help(reg))
        .subcommands(children(reg, ""));
    root.build();
    root
}

/// The subcommands directly under `prefix` ("" for the root, "vault" for
/// `vault`), in registry order. A registered name is a leaf; a segment
/// that only prefixes registered names (`vault` of `vault list`) is a
/// parent holding them.
fn children(reg: &Registry, prefix: &str) -> Vec<Command> {
    let under = |name: &'static str| -> Option<&'static str> {
        if prefix.is_empty() {
            Some(name)
        } else {
            name.strip_prefix(prefix)?.strip_prefix(' ')
        }
    };
    let mut segments: Vec<&'static str> = Vec::new();
    for info in &reg.commands {
        if let Some(rest) = under(info.name) {
            let segment = rest.split(' ').next().unwrap_or(rest);
            if !segments.contains(&segment) {
                segments.push(segment);
            }
        }
    }
    segments
        .into_iter()
        .map(|segment| {
            let full = if prefix.is_empty() {
                segment.to_string()
            } else {
                format!("{prefix} {segment}")
            };
            match reg.commands.iter().find(|c| c.name == full) {
                Some(info) => leaf(info, segment),
                None => {
                    let short = reg.parent(&full).map(|p| p.short).unwrap_or("");
                    styled(Command::new(segment).about(short))
                        .subcommand_help_heading("Available Commands")
                        .subcommand_value_name("command")
                        .subcommands(children(reg, &full))
                }
            }
        })
        .collect()
}

fn styled(cmd: Command) -> Command {
    cmd.help_template(COMMAND_TEMPLATE)
        .disable_help_subcommand(true)
}

fn leaf(info: &CommandInfo, name: &'static str) -> Command {
    let mut cmd = styled(Command::new(name).about(info.short))
        .aliases(info.aliases.iter().copied())
        .hide(info.availability == Availability::SessionOnly);
    // A command's long description replaces its short one in its own help.
    // Baked into the template rather than set as clap's long_about, so
    // `-h`, `--help` and `gage help <cmd>` all render the same text.
    let about = crate::commands::long_about(info.command)
        .map(|long| format!("{long}\n"))
        .unwrap_or_else(|| "{about-with-newline}".into());
    let aliases = if info.aliases.is_empty() {
        String::new()
    } else {
        alias_section(info, name)
    };
    cmd = cmd.help_template(format!(
        "{about}\nUsage:\n  {{usage}}\n\n{aliases}{{all-args}}"
    ));
    if info.command == registry::Command::Session(SessionVerb::Help) {
        // The topic is a command path, `help vault list`.
        return cmd.arg(
            Arg::new("command")
                .num_args(0..)
                .action(ArgAction::Append)
                .trailing_var_arg(true),
        );
    }
    for arg in usage_args(info.usage)
        .into_iter()
        .chain(crate::commands::flags(info.command))
    {
        cmd = cmd.arg(arg);
    }
    cmd
}

/// `Aliases:` with the canonical name and its aliases, sorted.
fn alias_section(info: &CommandInfo, name: &str) -> String {
    let mut names: Vec<&str> = std::iter::once(name)
        .chain(info.aliases.iter().copied())
        .collect();
    names.sort_unstable();
    format!("Aliases:\n  {}\n\n", names.join(", "))
}

/// Positional arguments from a registry usage string: `<x>` is required,
/// `[x]` optional, and a trailing `...` takes the rest.
fn usage_args(usage: &str) -> Vec<Arg> {
    usage
        .split_whitespace()
        .map(|token| {
            // `...` may sit inside the brackets (`[ID...]`)
            // or after them.
            let required = token.starts_with('<');
            let inner = token
                .trim_end_matches("...")
                .trim_matches(|c| matches!(c, '<' | '>' | '[' | ']'));
            let many = token.ends_with("...") || inner.ends_with("...");
            let name = inner.trim_end_matches("...");
            Arg::new(name.to_string())
                .required(required)
                .action(if many {
                    ArgAction::Append
                } else {
                    ArgAction::Set
                })
                .num_args(if many {
                    ValueRange::new(0..)
                } else {
                    ValueRange::new(1)
                })
        })
        .collect()
}

/// `gage --help` and `gage help`.
pub fn render_root_help(reg: &Registry) -> String {
    let mut lines = vec![
        ROOT_ABOUT.to_string(),
        String::new(),
        "Usage:".into(),
        "  gage [command]".into(),
        String::new(),
    ];
    for group in crate::registry::group_commands(&reg.one_shot()) {
        lines.push(format!("{}:", group.group));
        lines.extend(
            group
                .commands
                .iter()
                .map(|c| format!("  {}", command_list_line(c))),
        );
        lines.push(String::new());
    }
    lines.push(ROOT_FOOTER.into());
    lines.join("\n") + "\n"
}

/// In-session `help`: the same grouped listing from the same registry,
/// filtered to what a session can run (its own verbs and every command
/// available in both modes). Vault selection shows as `use <vault>`, not
/// the `-u/--use` flag, which per-command help documents
/// (ADR-0031).
pub fn render_session_help(reg: &Registry) -> String {
    let mut lines = vec!["gage session commands".to_string(), String::new()];
    for group in crate::registry::group_commands(&reg.session()) {
        lines.push(format!("{}:", group.group));
        lines.extend(
            group
                .commands
                .iter()
                .map(|c| format!("  {}", command_list_line(c))),
        );
        lines.push(String::new());
    }
    lines.push(r#"Type "help <command>" for more information about a command."#.into());
    lines.join("\n") + "\n"
}

/// One command's line in a grouped listing: its name with argument syntax,
/// aliases beside it, then the description. The two-space gap is kept for
/// any name length, so the name field always has a clear boundary.
pub fn command_list_line(info: &CommandInfo) -> String {
    let names: Vec<String> = std::iter::once(info.usage_name())
        .chain(info.aliases.iter().map(|a| a.to_string()))
        .collect();
    format!("{:<24}  {}", names.join(", "), info.short)
}

/// Parses one-shot `args` (without the program name) against `reg`.
pub fn parse(
    reg: &Registry,
    build_info: &BuildInfo,
    args: &[String],
    stdin_is_terminal: bool,
) -> Result<Outcome, CliError> {
    let mut cli = build(reg, build_info);
    let argv = std::iter::once("gage".to_string()).chain(args.iter().cloned());
    let matches = match cli.try_get_matches_from_mut(argv) {
        Ok(m) => m,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    Ok(Outcome::Print(e.to_string()))
                }
                _ => Err(usage_error(&cli, reg, args, &e)),
            };
        }
    };
    let script = script_source(&matches)?;
    let Some((name, sub)) = matches.subcommand() else {
        if let Some(source) = script {
            // `--yes` is for a run with nobody to show a recipient diff
            // to, which is exactly this; it is refused only where a human
            // is at the session.
            return Ok(Outcome::Script(ScriptRun {
                source,
                assume_yes: crate::assumeyes::given(&matches),
            }));
        }
        // Bare `gage --yes`: on the way into a session on a terminal, and
        // It is refused the same way when there is none.
        if crate::assumeyes::given(&matches) {
            return Err(crate::assumeyes::refused_in_session());
        }
        return Ok(if stdin_is_terminal {
            Outcome::Session
        } else {
            Outcome::Print(render_root_help(reg))
        });
    };
    if script.is_some() {
        // They choose where a whole session's lines come from, so with a
        // command they ask for a session and a one-shot in one breath.
        // Refusing beats picking one and ignoring half of what was typed.
        return Err(CliError::new(
            ExitCode::Usage,
            format!(
                "--script/--stdin run a whole session's commands, so they can't be \
                 combined with the {name:?} command; put {name} on a line in the script instead"
            ),
        ));
    }
    resolve(&cli, reg, vec![name.to_string()], sub)
}

/// The script source the flags name, if any. Both at once has no answer:
/// there would be two command streams.
fn script_source(matches: &ArgMatches) -> Result<Option<ScriptSource>, CliError> {
    let file = matches.get_one::<String>(SCRIPT_FLAG);
    let stdin = matches.get_flag(STDIN_FLAG);
    match (file, stdin) {
        (Some(_), true) => Err(CliError::new(
            ExitCode::Usage,
            "--script and --stdin are mutually exclusive",
        )),
        (Some(file), false) => Ok(Some(ScriptSource::File(file.clone()))),
        (None, true) => Ok(Some(ScriptSource::Stdin)),
        (None, false) => Ok(None),
    }
}

/// Descends to the leaf the matches name. A parent with no subcommand
/// prints its own help.
fn resolve(
    root: &Command,
    reg: &Registry,
    mut path: Vec<String>,
    matches: &ArgMatches,
) -> Result<Outcome, CliError> {
    if let Some((name, sub)) = matches.subcommand() {
        path.push(name.to_string());
        return resolve(root, reg, path, sub);
    }
    let full = path.join(" ");
    match reg.find(&full) {
        Some(info) => Ok(Outcome::Command(info.command, matches.clone())),
        None => Ok(Outcome::Print(command_help(root, reg, &path))),
    }
}

/// The help for the command at `path` (canonical names, as
/// [`reached_path`] returns), or the root listing for an empty path. `root`
/// must come from [`build`], so usage lines carry the full `gage ...` path.
pub fn command_help(root: &Command, reg: &Registry, path: &[String]) -> String {
    let mut cmd = root;
    for name in path {
        match cmd.find_subcommand(name) {
            Some(sub) => cmd = sub,
            None => break,
        }
    }
    if std::ptr::eq(cmd, root) {
        return render_root_help(reg);
    }
    cmd.clone().render_help().to_string()
}

/// Follows `args` through the tree by subcommand names and aliases,
/// skipping flags, and returns the canonical path it reached.
pub fn reached_path(root: &Command, args: &[String]) -> Vec<String> {
    let mut cmd = root;
    let mut path = Vec::new();
    for arg in args.iter().filter(|a| !a.starts_with('-')) {
        match cmd.find_subcommand(arg) {
            Some(sub) => {
                path.push(sub.get_name().to_string());
                cmd = sub;
            }
            None => break,
        }
    }
    path
}

/// A parse rejection (unknown command or flag, wrong argument count),
/// carrying the failing command's own help so nobody is left with only
/// clap's complaint. Coded Usage. Only parse rejections get this:
/// application errors keep their own message.
fn usage_error(root: &Command, reg: &Registry, args: &[String], e: &clap::Error) -> CliError {
    let rendered = e.to_string();
    let complaint = rendered
        .split("\nUsage:")
        .next()
        .unwrap_or(&rendered)
        .trim()
        .trim_start_matches("error: ");
    let help = command_help(root, reg, &reached_path(root, args));
    CliError::new(
        ExitCode::Usage,
        format!("{complaint}\n\n{}", help.trim_end()),
    )
}

/// `gage help [command...]`: the root listing, or one command's help. An
/// unknown topic is a usage error.
pub fn help_topic(
    reg: &Registry,
    build_info: &BuildInfo,
    topic: &[String],
) -> Result<String, CliError> {
    let root = build(reg, build_info);
    if topic.is_empty() {
        return Ok(render_root_help(reg));
    }
    let path = reached_path(&root, topic);
    if path.len() != topic.len() {
        return Err(CliError::new(
            ExitCode::Usage,
            format!("unknown help topic {:?}", topic.join(" ")),
        ));
    }
    Ok(command_help(&root, reg, &path))
}
