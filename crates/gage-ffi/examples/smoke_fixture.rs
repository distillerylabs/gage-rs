//! Writes the vault the Swift smoke test reads (`make ios-smoke`): a git
//! vault named `personal` with two entries, under `<root>/config`,
//! `<root>/data` and `<root>/state`, made through the same
//! `Vault::init` an app calls.
//!
//! Usage: `smoke_fixture <root>`. The passphrase is fixed and public, since
//! the vault holds nothing real; `tests/swift/main.swift` knows it.

// An example is a frontend like gage-cli: printing is its job.
#![allow(clippy::print_stdout)]

use std::path::PathBuf;

use gage::entry::NewEntry;
use gage::prompter::FakePrompter;
use gage::storage::xdgpaths::Dirs;
use gage::{InitSpec, UnlockMethod, Vault, VaultType};

const PASSPHRASE: &str = "correct horse battery staple";
const TITLES: [&str; 2] = ["alpha", "bravo"];

fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: smoke_fixture <root>"),
    );
    let dirs = Dirs {
        config: root.join("config"),
        data: root.join("data"),
        state: root.join("state"),
    };
    let prompter = FakePrompter::new().with_passphrase(PASSPHRASE);
    let made = Vault::init(
        &dirs,
        &InitSpec {
            name: "personal".into(),
            path: root.join("personal"),
            vault_type: VaultType::Git,
            method: UnlockMethod::Passphrase,
            device: "smoke-device".into(),
            recipients: Vec::new(),
            recovery_key: None,
            remote: None,
        },
        &prompter,
    )
    .expect("creating the vault");

    let identity = made
        .vault
        .unlock(&FakePrompter::new().with_passphrase(PASSPHRASE))
        .expect("unlocking the new vault");
    for title in TITLES {
        made.vault
            .insert(
                &identity,
                NewEntry {
                    title: title.into(),
                    description: format!("about {title}"),
                    value: format!("secret-{title}"),
                    ..NewEntry::default()
                },
                true,
                &FakePrompter::new(),
            )
            .expect("inserting an entry");
    }
    println!("{}", root.display());
}
