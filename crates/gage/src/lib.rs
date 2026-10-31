//! The library: everything that decides what gage does.
//!
//! # Using it
//!
//! This is the `gage-core` crate, imported as `gage`. It is the library under
//! the `gage` command-line tool, for building another frontend over the same
//! vaults. A [`Vault`] is one vault's on-disk state and does the vault-wide
//! work (create, list, add a recipient). [`Vault::unlock`] turns a
//! [`Prompter`], which you implement to ask a human, into an [`Identity`],
//! and every call that reads or writes an entry takes that identity
//! explicitly. A [`Session`] holds one or more unlocked vaults for a
//! process's lifetime. Anything that needs a human decision comes back as a
//! typed value (a candidate list, a [`Warning`]) rather than as output, and
//! every error carries an [`ErrorKind`] you can match on.
//!
//! The crate is pre-1.0: a minor release may change this API. The on-disk
//! format has its own version (`doc/format/` in the repository) and does not
//! move with the crate's.
//!
//! `gage-cli` is a thin frontend over this crate, and a future GUI/TUI is a
//! second one. So nothing here may touch the terminal or the process: see
//! `clippy.toml` and the denies below, and `tests/no_frontend_deps.rs` for
//! the dependency side of the same rule.
//!
//! The crate is organised by concern (ADR-0055):
//!
//! - [`vault`] and [`session`]: the two things a frontend drives
//! - [`entry`]: a decrypted record, and finding, listing and moving them
//! - [`trust`]: recipients, the trust cache, orphans
//! - [`enrollment`] and [`recovery`]: getting a device in, getting back in
//! - [`sync`]: the git remote
//! - [`crypto`]: keys, ciphertext, the in-memory identity
//! - [`storage`]: what gage keeps on disk
//! - [`prompter`]: every question the library asks a human
//! - [`error`]: the frontend-neutral error kind

#![deny(clippy::disallowed_macros, clippy::disallowed_methods)]

pub mod crypto;
pub mod devicename;
pub mod enrollment;
pub mod entry;
pub mod error;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod gittest;
pub mod prompter;
pub mod recovery;
pub mod session;
pub mod storage;
pub mod sync;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod testsupport;
pub mod trust;
pub mod vault;

pub use crate::crypto::identity::Identity;
pub use crate::entry::Entry;
pub use crate::error::{ErrorKind, HasKind, Remedy};
pub use crate::prompter::{Prompter, Warning};
pub use crate::session::{Clock, Session, SessionConfig, SystemClock};
pub use crate::storage::vaultconfig::{UnlockMethod, VaultId, VaultType};
pub use crate::vault::{CreateSpec, Vault};

#[cfg(test)]
mod format_v2_bytes_tests;
#[cfg(test)]
mod format_v2_tests;
#[cfg(test)]
mod session_tests;
