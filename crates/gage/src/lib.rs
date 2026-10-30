//! The library: everything that decides what gage does.
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
