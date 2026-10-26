//! Reading the Go help fixtures (`tests/fixtures/go/help/*.txt`), shared by
//! every test that compares gage-rs's command surface to Go's. The fixtures
//! are a reference for name, aliases, group, arguments and flags, not a byte
//! gate (Q-HELP-RENDER), so this parses only those.

use std::path::PathBuf;

/// One flag in a fixture's `Flags:` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagSpec {
    pub short: Option<char>,
    pub long: Option<String>,
    /// Go prints a value type (`string`, `stringArray`) after a flag that
    /// takes one.
    pub takes_value: bool,
}

/// A positional argument from a fixture's `Usage:` line, lower-cased: Go
/// writes `[QUERY]` where gage-rs writes `[query]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Positional {
    pub name: String,
    pub required: bool,
    pub many: bool,
}

/// The directory the fixtures live in.
pub fn help_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/go/help")
}

/// Every fixture as `(file stem, text)`, sorted by name. Read at run time so
/// a fixture nobody references still gets checked.
pub fn all_fixtures() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(help_dir())
        .expect("the help fixture directory is readable")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .map(|p| {
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).expect("fixture is UTF-8");
            (stem, text)
        })
        .collect();
    out.sort();
    out
}

/// The command path a fixture documents: `vault_set-default` is
/// `vault set-default`. `-` is part of a name, never a separator. `root` is
/// the root command, whose path is empty.
pub fn fixture_path(stem: &str) -> Vec<String> {
    if stem == "root" {
        return Vec::new();
    }
    stem.split('_').map(str::to_string).collect()
}

/// The names on a fixture's `Aliases:` line. Cobra lists the command's own
/// name among them, so callers drop it.
pub fn fixture_aliases(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|l| l.trim_end() != "Aliases:")
        .nth(1)
        .map(|l| l.split(',').map(|n| n.trim().to_string()).collect())
        .unwrap_or_default()
}

/// The flags in a fixture's `Flags:` block, except Cobra's own `-h/--help`.
pub fn fixture_flag_specs(help: &str) -> Vec<FlagSpec> {
    help.lines()
        .skip_while(|l| l.trim_end() != "Flags:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let mut words = line.split_whitespace().peekable();
            let (mut short, mut long) = (None, None);
            while let Some(w) = words.next_if(|w| w.starts_with('-')) {
                let w = w.trim_end_matches(',');
                if let Some(l) = w.strip_prefix("--") {
                    long = Some(l.to_string());
                } else {
                    short = w.chars().nth(1);
                }
            }
            if short.is_none() && long.is_none() {
                return None;
            }
            // The next word is a value type, or the start of the description
            // when the flag takes none; types are single lower-camel words
            // and Go's descriptions are separated from them by 2+ spaces.
            let takes_value = line
                .trim_start()
                .split("  ")
                .next()
                .is_some_and(|names| names.split_whitespace().any(|w| !w.starts_with('-')));
            Some(FlagSpec {
                short,
                long,
                takes_value,
            })
        })
        .filter(|f| f.long.as_deref() != Some("help"))
        .collect()
}

/// The long and short flag spellings in a fixture's `Flags:` section.
pub fn fixture_flags(help: &str) -> Vec<String> {
    fixture_flag_specs(help)
        .into_iter()
        .flat_map(|f| {
            f.short
                .map(|c| format!("-{c}"))
                .into_iter()
                .chain(f.long.map(|l| format!("--{l}")))
        })
        .collect()
}

/// The positionals on a fixture's `Usage:` line, in order. `[flags]`,
/// `[command]`, flag names and the values of flags that take one are not
/// positionals.
pub fn fixture_positionals(help: &str) -> Vec<Positional> {
    let specs = fixture_flag_specs(help);
    let usage = help
        .lines()
        .skip_while(|l| l.trim_end() != "Usage:")
        .nth(1)
        .unwrap_or("");
    let mut out = Vec::new();
    // `gage`, then the command path, then arguments.
    let mut after_value_flag = false;
    for token in usage.split_whitespace().skip(1) {
        if let Some(flag) = token.strip_prefix("--") {
            after_value_flag = specs
                .iter()
                .any(|f| f.long.as_deref() == Some(flag) && f.takes_value);
            continue;
        }
        let required = token.starts_with('<');
        if !required && !token.starts_with('[') || token == "[flags]" || token == "[command]" {
            continue;
        }
        if std::mem::take(&mut after_value_flag) {
            continue;
        }
        let many = token.contains("...");
        let name = token
            .trim_matches(|c| matches!(c, '<' | '>' | '[' | ']' | '.'))
            .to_lowercase();
        out.push(Positional {
            name,
            required,
            many,
        });
    }
    out
}

/// The child names under a parent fixture's `Available Commands:`.
pub fn fixture_children(help: &str) -> Vec<String> {
    help.lines()
        .skip_while(|l| l.trim_end() != "Available Commands:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

/// `root.txt`'s grouped listing as `(group, names)`, one entry per listed
/// command, its names being the canonical command path first and then any
/// aliases (`search, grep`).
pub fn root_listing(root: &str) -> Vec<(String, Vec<String>)> {
    let mut group = None;
    let mut out = Vec::new();
    for line in root.lines().map(str::trim_end) {
        if let Some(heading) = line.strip_suffix(':').filter(|h| !h.starts_with(' ')) {
            // Only the grouped sections: "Usage:" precedes them.
            group = (heading != "Usage").then(|| heading.to_string());
        } else if line.starts_with("  ") {
            let Some(group) = &group else { continue };
            let names = line.trim_start().split("  ").next().unwrap_or("");
            out.push((
                group.clone(),
                names.split(", ").map(|n| n.trim().to_string()).collect(),
            ));
        } else if line.trim().is_empty() {
            // A blank line closes the section.
            group = None;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_mapping_keeps_hyphens_and_maps_root_to_nothing() {
        assert_eq!(fixture_path("root"), Vec::<String>::new());
        assert_eq!(fixture_path("insert"), ["insert"]);
        assert_eq!(fixture_path("vault_set-default"), ["vault", "set-default"]);
        assert_eq!(fixture_path("auth_login"), ["auth", "login"]);
    }

    #[test]
    fn the_aliases_line_is_read_with_the_commands_own_name() {
        let search = "Find\n\nUsage:\n  gage search <pattern> [flags]\n\nAliases:\n  grep, search\n\nFlags:\n  -u, --use string   vault\n";
        assert_eq!(fixture_aliases(search), ["grep", "search"]);
        assert!(fixture_aliases("Create\n\nUsage:\n  gage init <name>\n").is_empty());
    }

    #[test]
    fn flags_are_read_short_and_long_with_and_without_a_value() {
        let help = "x\n\nUsage:\n  gage insert <title> [flags]\n\nFlags:\n      --description string   optional\n  -e, --edit                 open a template\n  -h, --help                 help for insert\n  -x                         short only\n";
        assert_eq!(
            fixture_flag_specs(help),
            [
                FlagSpec {
                    short: None,
                    long: Some("description".into()),
                    takes_value: true
                },
                FlagSpec {
                    short: Some('e'),
                    long: Some("edit".into()),
                    takes_value: false
                },
                FlagSpec {
                    short: Some('x'),
                    long: None,
                    takes_value: false
                },
            ],
            "-h/--help is Cobra's own and is dropped"
        );
        assert_eq!(fixture_flags(help), ["--description", "-e", "--edit", "-x"]);
        assert!(fixture_flags("x\n\nUsage:\n  gage vault\n").is_empty());
    }

    #[test]
    fn positionals_skip_flags_their_values_and_the_catch_alls() {
        let p = |name: &str, required, many| Positional {
            name: name.into(),
            required,
            many,
        };
        let cp = "x\n\nUsage:\n  gage cp <query> --to-vault <name> [flags]\n\nFlags:\n      --to-vault string   dest\n";
        assert_eq!(fixture_positionals(cp), [p("query", true, false)]);
        // A boolean flag does not swallow the argument after it.
        let history = "x\n\nUsage:\n  gage history --decrypt <query> [flags]\n\nFlags:\n      --decrypt      required\n  -u, --use string   vault\n";
        assert_eq!(fixture_positionals(history), [p("query", true, false)]);
        let approve = "x\n\nUsage:\n  gage recipient approve [ID...] [flags]\n";
        assert_eq!(fixture_positionals(approve), [p("id", false, true)]);
        let log = "x\n\nUsage:\n  gage log [QUERY] [flags]\n";
        assert_eq!(fixture_positionals(log), [p("query", false, false)]);
        let root = "x\n\nUsage:\n  gage [command]\n";
        assert!(fixture_positionals(root).is_empty());
    }

    #[test]
    fn a_parents_children_and_the_root_listing_are_read() {
        let vault = "Manage\n\nUsage:\n  gage vault\n\nAvailable Commands:\n  info            Show\n  set-default     Change\n";
        assert_eq!(fixture_children(vault), ["info", "set-default"]);
        let root = "gage\n\nUsage:\n  gage [command]\n\nVault lifecycle:\n  init                      Create\n  vault list                List\n\nEntry CRUD:\n  search, grep              Find\n\nUse \"gage help\"\n";
        assert_eq!(
            root_listing(root),
            [
                ("Vault lifecycle".to_string(), vec!["init".to_string()]),
                (
                    "Vault lifecycle".to_string(),
                    vec!["vault list".to_string()]
                ),
                (
                    "Entry CRUD".to_string(),
                    vec!["search".to_string(), "grep".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn the_fixture_directory_is_found() {
        assert!(
            all_fixtures().iter().any(|(stem, _)| stem == "root"),
            "{:?}",
            help_dir()
        );
    }
}
