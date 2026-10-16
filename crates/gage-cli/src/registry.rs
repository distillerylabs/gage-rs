//! The command registry: every command's name, aliases, description, group
//! and one-shot/session availability, in one place. One-shot help (clap)
//! and M6's in-session help both render from it, so the two can't drift,
//! and the clap command tree is built from it (see `cli`). Adding a command
//! means registering it here, not just adding a clap subcommand.

/// Where a command can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Both,
    OneShotOnly,
    SessionOnly,
}

impl Availability {
    pub fn one_shot_visible(self) -> bool {
        matches!(self, Availability::Both | Availability::OneShotOnly)
    }

    pub fn session_visible(self) -> bool {
        matches!(self, Availability::Both | Availability::SessionOnly)
    }
}

pub const GROUP_SESSION: &str = "Session";
pub const GROUP_VAULT: &str = "Vault lifecycle";
pub const GROUP_IDENTITY: &str = "Identity";
pub const GROUP_RECIPIENT: &str = "Recipients";
pub const GROUP_ENTRY: &str = "Entry CRUD";
pub const GROUP_SYNC: &str = "Sync";
pub const GROUP_GIT: &str = "Git-specific";

/// The order groups render in. A group missing from it still renders, after
/// these (see [`group_commands`]).
pub const GROUP_ORDER: [&str; 7] = [
    GROUP_SESSION,
    GROUP_VAULT,
    GROUP_IDENTITY,
    GROUP_RECIPIENT,
    GROUP_ENTRY,
    GROUP_SYNC,
    GROUP_GIT,
];

/// One invocable command. `name` is its full path below `gage`, so a nested
/// command is `"vault list"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub short: &'static str,
    pub group: &'static str,
    /// Argument syntax after the name (`"<vault>"`, `"[command]"`), shared
    /// by the usage line and the session listing.
    pub usage: &'static str,
    pub availability: Availability,
}

impl CommandInfo {
    /// The name with its argument syntax: `use <vault>`.
    pub fn usage_name(&self) -> String {
        if self.usage.is_empty() {
            self.name.to_string()
        } else {
            format!("{} {}", self.name, self.usage)
        }
    }
}

/// A command that only groups subcommands (`vault`, `git`). Not invocable
/// itself, so not a [`CommandInfo`]; running it shows its help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentInfo {
    pub name: &'static str,
    pub short: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    pub commands: Vec<CommandInfo>,
    pub parents: Vec<ParentInfo>,
}

/// The real registry. Commands are listed in the order Go's registry
/// lists them, which is the order each group renders in.
pub fn registry() -> Registry {
    use Availability::{Both, OneShotOnly, SessionOnly};
    let session = |name, aliases, usage, short| CommandInfo {
        name,
        aliases,
        short,
        group: GROUP_SESSION,
        usage,
        availability: SessionOnly,
    };
    let command = |name, usage, short, group, availability| CommandInfo {
        name,
        aliases: &[],
        short,
        group,
        usage,
        availability,
    };
    Registry {
        commands: vec![
            session(
                "use",
                &[],
                "<vault>",
                "Switch/unlock the active vault for this session",
            ),
            session(
                "lock",
                &[],
                "[vault]",
                "Drop key material for one vault (or all) without exiting",
            ),
            session(
                "status",
                &["whoami"],
                "",
                "List vaults touched this session and their lock state",
            ),
            session("exit", &["quit"], "", "Leave the session"),
            session("help", &[], "[command]", "Show help for gage's commands"),
            // One-shot only: it creates a vault rather than operating on
            // one, which leaves "does the new vault become the session's
            // current one?" unanswered (Go's Q-CMD-AVAILABILITY).
            command(
                "init",
                "<name>",
                "Create a new vault",
                GROUP_VAULT,
                OneShotOnly,
            ),
            // One-shot only, like init: it creates a vault rather than
            // operating on one.
            command(
                "clone",
                "<remote-url>",
                "Clone an existing vault from a remote",
                GROUP_VAULT,
                OneShotOnly,
            ),
            command(
                "vault list",
                "",
                "List registered vaults",
                GROUP_VAULT,
                Both,
            ),
            command(
                "vault info",
                "[name]",
                "Show a vault's type, method, recipient count, and (for git) remote/status",
                GROUP_VAULT,
                Both,
            ),
            command(
                "vault remove",
                "<name>",
                "Forget a vault locally, leaving its files untouched",
                GROUP_VAULT,
                Both,
            ),
            command(
                "vault set-default",
                "<name>",
                "Change which vault is used when none is given",
                GROUP_VAULT,
                Both,
            ),
            command(
                "identity add",
                "",
                "Register this device's identity and print its public key",
                GROUP_IDENTITY,
                Both,
            ),
            command(
                "identity list",
                "",
                "List the keys this machine holds for a vault",
                GROUP_IDENTITY,
                Both,
            ),
            command(
                "recipient add",
                "<pubkey>",
                "Authorize a public key and re-encrypt the vault to include it",
                GROUP_RECIPIENT,
                Both,
            ),
            command(
                "recipient remove",
                "<pubkey-or-name>",
                "Revoke a recipient and re-encrypt the vault without its key",
                GROUP_RECIPIENT,
                Both,
            ),
            command(
                "recipient list",
                "",
                "List the keys this vault is encrypted to",
                GROUP_RECIPIENT,
                Both,
            ),
            command(
                "recipient verify",
                "",
                "Check .age-recipients and config.toml still agree",
                GROUP_RECIPIENT,
                Both,
            ),
            command("insert", "<title>", "Create a new entry", GROUP_ENTRY, Both),
            command(
                "show",
                "<query>",
                "Print an entry's value",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "cat",
                "<query>",
                "Print an entry's full decrypted contents",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "edit",
                "<query>",
                "Edit an entry's full YAML in $EDITOR",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "rename",
                "<query> <new-title>",
                "Change an entry's title",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "generate",
                "<title>",
                "Create a new entry with a randomly generated value",
                GROUP_ENTRY,
                Both,
            ),
            command("rm", "<query>", "Delete an entry", GROUP_ENTRY, Both),
            command(
                "mv",
                "<query>",
                "Move an entry to another vault — the sharing mechanism",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "cp",
                "<query>",
                "Copy an entry into another vault, keeping the original",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "ls",
                "",
                "List entries with their ids, dates, and last writer",
                GROUP_ENTRY,
                Both,
            ),
            CommandInfo {
                name: "search",
                aliases: &["grep"],
                short: "Find entries by title, description, or body text",
                group: GROUP_ENTRY,
                usage: "<pattern>",
                availability: Both,
            },
            command(
                "reindex",
                "",
                "Force a session's cached metadata index to rebuild",
                GROUP_ENTRY,
                Both,
            ),
            command(
                "sync",
                "",
                "Pull, merge what can be merged, then push",
                GROUP_SYNC,
                Both,
            ),
            command(
                "pull",
                "",
                "Fast-forward this vault from its remote",
                GROUP_SYNC,
                Both,
            ),
            command(
                "push",
                "",
                "Publish this vault's commits to its remote",
                GROUP_SYNC,
                Both,
            ),
            command(
                "log",
                "[query]",
                "Show an entry's commit history — timestamps only, nothing decrypted",
                GROUP_SYNC,
                Both,
            ),
            command(
                "history",
                "<query>",
                "Decrypt every past revision of one entry (surfaces old secret values)",
                GROUP_SYNC,
                Both,
            ),
            command(
                "git set-remote",
                "<name> <url>",
                "Set or change a vault's git remote (origin)",
                GROUP_GIT,
                Both,
            ),
            command(
                "auth login",
                "",
                "Store a token for a git host",
                GROUP_GIT,
                Both,
            ),
            command(
                "auth status",
                "",
                "Show which git hosts have a token",
                GROUP_GIT,
                Both,
            ),
            command(
                "auth logout",
                "",
                "Forget a git host's token",
                GROUP_GIT,
                Both,
            ),
        ],
        parents: vec![
            ParentInfo {
                name: "vault",
                short: "Manage registered vaults",
            },
            ParentInfo {
                name: "identity",
                short: "Manage this device's identities for a vault",
            },
            ParentInfo {
                name: "recipient",
                short: "Manage who can read this vault",
            },
            ParentInfo {
                name: "git",
                short: "Git-specific vault operations",
            },
            ParentInfo {
                name: "auth",
                short: "Manage tokens for git hosts",
            },
        ],
    }
}

impl Registry {
    /// Resolves a command by its full name or any alias.
    pub fn find(&self, name: &str) -> Option<&CommandInfo> {
        self.commands
            .iter()
            .find(|c| c.name == name || c.aliases.contains(&name))
    }

    pub fn parent(&self, name: &str) -> Option<&ParentInfo> {
        self.parents.iter().find(|p| p.name == name)
    }

    pub fn one_shot(&self) -> Vec<CommandInfo> {
        self.filter(Availability::one_shot_visible)
    }

    pub fn session(&self) -> Vec<CommandInfo> {
        self.filter(Availability::session_visible)
    }

    fn filter(&self, visible: fn(Availability) -> bool) -> Vec<CommandInfo> {
        self.commands
            .iter()
            .filter(|c| visible(c.availability))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandGroup {
    pub group: &'static str,
    pub commands: Vec<CommandInfo>,
}

/// Groups commands in [`GROUP_ORDER`], keeping registry order within a
/// group. A group missing from the order still renders, after the known
/// ones, rather than silently dropping its commands.
pub fn group_commands(commands: &[CommandInfo]) -> Vec<CommandGroup> {
    let mut groups: Vec<CommandGroup> = Vec::new();
    let known = GROUP_ORDER.iter().copied();
    let unknown = commands
        .iter()
        .map(|c| c.group)
        .filter(|g| !GROUP_ORDER.contains(g));
    for group in known.chain(unknown) {
        if groups.iter().any(|g| g.group == group) {
            continue;
        }
        let members: Vec<CommandInfo> = commands
            .iter()
            .filter(|c| c.group == group)
            .cloned()
            .collect();
        if !members.is_empty() {
            groups.push(CommandGroup {
                group,
                commands: members,
            });
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go: TestFindCommandResolvesAliases
    #[test]
    fn find_command_resolves_aliases() {
        assert_eq!(registry().find("whoami").unwrap().name, "status");
        assert_eq!(registry().find("quit").unwrap().name, "exit");
    }

    // Go: TestFindCommandUnknown
    #[test]
    fn find_command_unknown() {
        assert_eq!(registry().find("not-a-command"), None);
    }

    // Go: TestEveryRegistryEntryHasShortAndGroup. Availability can't be
    // unset in Rust (a closed enum with no default), so only the strings
    // need checking.
    #[test]
    fn every_registry_entry_has_short_and_group() {
        let reg = registry();
        for c in &reg.commands {
            assert!(!c.short.trim().is_empty(), "{} has no short", c.name);
            assert!(!c.group.trim().is_empty(), "{} has no group", c.name);
        }
        for p in &reg.parents {
            assert!(!p.short.trim().is_empty(), "{} has no short", p.name);
        }
    }

    #[test]
    fn names_and_aliases_are_unique() {
        let reg = registry();
        let mut seen = std::collections::HashSet::new();
        for c in &reg.commands {
            for n in std::iter::once(&c.name).chain(c.aliases) {
                assert!(seen.insert(*n), "{n} registered twice");
            }
        }
    }

    // Go: TestGroupCommandsStillRendersAGroupMissingFromGroupOrder
    #[test]
    fn group_commands_still_renders_a_group_missing_from_group_order() {
        let cmd = |name, group| CommandInfo {
            name,
            aliases: &[],
            short: "s",
            group,
            usage: "",
            availability: Availability::Both,
        };
        let got = group_commands(&[
            cmd("orphan-cmd", "zzz-not-in-group-order"),
            cmd("known-cmd", GROUP_ORDER[0]),
            cmd("orphan-two", "zzz-not-in-group-order"),
        ]);
        let groups: Vec<&str> = got.iter().map(|g| g.group).collect();
        assert_eq!(groups, [GROUP_ORDER[0], "zzz-not-in-group-order"]);
        assert_eq!(got[1].commands.len(), 2, "rendered once, with both members");
    }

    #[test]
    fn groups_render_in_group_order() {
        let cmd = |name, group| CommandInfo {
            name,
            aliases: &[],
            short: "s",
            group,
            usage: "",
            availability: Availability::Both,
        };
        let got = group_commands(&[cmd("e", GROUP_ENTRY), cmd("v", GROUP_VAULT)]);
        let groups: Vec<&str> = got.iter().map(|g| g.group).collect();
        assert_eq!(groups, [GROUP_VAULT, GROUP_ENTRY]);
    }
}
