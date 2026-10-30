//! What gage keeps on disk: atomic writes, the global config, the vault
//! config, the XDG paths, the per-vault lock and the identity files.

pub mod atomicfile;
pub mod config;
pub mod identityfile;
pub mod paths;
pub(crate) mod serde_opt;
pub mod vaultconfig;
#[cfg(any(test, feature = "test-support"))]
pub mod vaultlock;
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) mod vaultlock;
pub mod xdgpaths;
