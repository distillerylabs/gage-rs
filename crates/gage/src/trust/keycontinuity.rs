//! Key continuity (ADR-0067): this device's record of the vault key it last
//! saw, and the check every use of the key goes through.
//!
//! Anyone can seal a key to the public recipient list, so the key file
//! proves nothing by itself. A file that continues the key this device
//! recorded does: its current or retired keys include that key, and only a
//! holder of it can write such a file.
//!
//! It is a file of its own beside `trust.toml`, not a field in it. Reads
//! advance this record without the vault's lock, and writes rewrite
//! `trust.toml` whole under it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::crypto::vaultkey::{Error, VAULT_KEY_FILE, VaultKey};
use crate::storage::{atomicfile, paths};
use crate::sync::gitrepo;
use crate::vault::Vault;

/// The record's name, under `$GAGE_STATE/<vault-id>/`.
pub const KEY_CONTINUITY_FILE: &str = "key-continuity.toml";

/// A fingerprint's length in hex: one HMAC-SHA256, spelled out.
const FINGERPRINT_HEX_LEN: usize = 64;

#[derive(Serialize, Deserialize)]
struct Record {
    fingerprint: String,
}

fn well_formed(fingerprint: &str) -> bool {
    fingerprint.len() == FINGERPRINT_HEX_LEN
        && fingerprint
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Vault {
    fn key_continuity_path(&self) -> PathBuf {
        paths::trust_cache_dir(self.dirs(), &self.id).join(KEY_CONTINUITY_FILE)
    }

    /// The fingerprint this device recorded, or `None` if it has none yet.
    /// A record that can't be read or understood is an error: read as
    /// absent, it would be a first use that trusts whatever is there.
    fn seen_key(&self) -> Result<Option<String>, Error> {
        let path = self.key_continuity_path();
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(Error::Io {
                    action: "reading",
                    path,
                    source,
                });
            }
        };
        match toml::from_str::<Record>(&String::from_utf8_lossy(&data)) {
            Ok(record) if well_formed(&record.fingerprint) => Ok(Some(record.fingerprint)),
            _ => Err(Error::BadRecord { path }),
        }
    }

    /// Records `key` as the one this device last saw.
    pub(crate) fn record_key(&self, key: &VaultKey) -> Result<(), Error> {
        let path = self.key_continuity_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| Error::Io {
                action: "creating",
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let record = Record {
            fingerprint: key.fingerprint(),
        };
        let text = toml::to_string(&record).expect("a string field always encodes");
        atomicfile::write(&path, text.as_bytes(), 0o600)
            .map_err(|source| Error::Write { path, source })
    }

    /// Refuses `key`, opened from the bytes `sealed`, unless it continues
    /// the key this device last saw. A device with no record takes it on
    /// first use.
    ///
    /// The record moves only to a published key: one still local can be
    /// discarded by a reset or lose a race with another device's rotation,
    /// and a record ahead of the vault would refuse it for good. Failing to
    /// move it doesn't fail the read: the key has already passed, and the
    /// record stays at one of its ancestors.
    pub(crate) fn check_key_continuity(&self, key: &VaultKey, sealed: &[u8]) -> Result<(), Error> {
        let seen = self.seen_key()?;
        if seen.as_deref() == Some(key.fingerprint().as_str()) {
            return Ok(());
        }
        if seen.is_some_and(|fingerprint| !key.descends_from(&fingerprint)) {
            return Err(Error::Discontinuous {
                vault: self.name.clone(),
                path: self.path.join(VAULT_KEY_FILE),
            });
        }
        if self.is_published(sealed) {
            let _ = self.record_key(key);
        }
        Ok(())
    }

    /// Whether `sealed` is the key file as published. Anything that can't
    /// be determined isn't.
    fn is_published(&self, sealed: &[u8]) -> bool {
        gitrepo::published_content(&self.path, VAULT_KEY_FILE)
            .is_ok_and(|published| published.as_deref() == Some(sealed))
    }
}
