//! The gage library: everything that decides what gage does.
//!
//! `gage-cli` is a thin frontend over this crate, and a future GUI/TUI is a
//! second one. So nothing here may touch the terminal or the process: see
//! `clippy.toml` and the denies below, and `tests/no_frontend_deps.rs` for
//! the dependency side of the same rule.

#![deny(clippy::disallowed_macros, clippy::disallowed_methods)]

pub mod agekey;
pub mod atomicfile;
pub mod config;
pub mod crypt;
pub mod devicename;
pub mod entry;
pub mod exitcode;
pub mod generate;
pub mod gitrepo;
#[cfg(any(test, feature = "test-support"))]
pub mod gittest;
pub mod identity;
pub mod identityfile;
pub mod locker;
pub mod memlock;
pub mod paths;
pub mod prompter;
pub mod recipients;
#[cfg(any(test, feature = "test-support"))]
pub mod testsupport;
pub mod vault;
pub mod vaultconfig;
pub mod vaultlock;
pub mod xdgpaths;

#[cfg(test)]
mod unlock_tests;
