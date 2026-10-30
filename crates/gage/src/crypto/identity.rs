//! A vault's unlocked private key on this device.
//!
//! `Identity` is a parameter, never internal state: `Vault::unlock` is the
//! only thing that produces one, and every CRUD method takes it explicitly,
//! so `Vault` stays stateless and identity-agnostic.
//!
//! Memory layout (ADR-0023): the key is held as its
//! `AGE-SECRET-KEY-1…` line in a `LockedBuf`, a page-aligned allocation
//! no other data shares, page-locked if the platform allows. The X25519
//! key age decrypts with is parsed from that line per operation and
//! dropped (age zeroizes it) straight after. `Drop` wipes the line, then
//! releases the page lock if one was taken, then frees the pages: there is
//! no separate close to forget, and nothing can use an `Identity` after it
//! is dropped, which the compiler checks:
//!
//! ```compile_fail,E0382
//! # fn f(vault: &gage::vault::Vault, p: &dyn gage::prompter::Prompter) {
//! let id = vault.unlock(p).unwrap();
//! drop(id);
//! let _ = gage::crypto::crypt::decrypt(b"", &id); // error: use of moved value
//! # }
//! ```
//!
//! Nor can two copies of one key exist to be dropped separately (`Identity` is
//! not `Clone`):
//!
//! ```compile_fail,E0599
//! # fn f(vault: &gage::vault::Vault, p: &dyn gage::prompter::Prompter) {
//! let id = vault.unlock(p).unwrap();
//! let copy = id.clone(); // error: `Identity` is not `Clone`
//! # }
//! ```
//!
//! Stable rustdoc doesn't check the error codes named above, so what pins
//! each failure to its named cause is this: the same code minus the
//! offending line compiles.
//!
//! ```no_run
//! # fn f(vault: &gage::vault::Vault, p: &dyn gage::prompter::Prompter) {
//! let id = vault.unlock(p).unwrap();
//! let _ = gage::crypto::crypt::decrypt(b"", &id);
//! drop(id);
//! # }
//! ```
//!
//! Recorded gap: age's Bech32 decoding makes
//! transient copies of the key while parsing it, which are neither locked
//! nor (all) wiped. The line this type owns end to end is.

use std::io;
use std::sync::Arc;

use zeroize::Zeroize;

use crate::crypto::locker::Locker;
use crate::crypto::memlock::LockedBuf;

/// The Bech32 human-readable part of an age private key, and how a key
/// line is found in a decrypted identity file.
pub(crate) const SECRET_KEY_PREFIX: &str = "AGE-SECRET-KEY-1";

/// An unlocked identity. It can't be constructed outside the library.
pub struct Identity {
    device: String,
    recipient: String,
    secret: LockedBuf,
    locker: Arc<dyn Locker>,
    page_locked: bool,
}

/// A key line that is not an age X25519 private key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub(crate) struct InvalidKey(pub &'static str);

/// One validated `AGE-SECRET-KEY-1…` line, copied into its own pages, and
/// the public key it derives: what an identity file yields once opened.
pub(crate) struct KeyLine {
    secret: LockedBuf,
    recipient: String,
}

impl KeyLine {
    /// Copies `line` into a `LockedBuf` and checks it parses. The copy
    /// comes first, so the caller can wipe the buffer `line` lives in
    /// straight after, whatever the result.
    pub(crate) fn parse(line: &[u8]) -> Result<KeyLine, InvalidKey> {
        let secret = LockedBuf::copy_from(line);
        let recipient = parse(&secret)?.to_public().to_string();
        Ok(KeyLine { secret, recipient })
    }

    pub(crate) fn recipient(&self) -> &str {
        &self.recipient
    }
}

/// How the one page-lock attempt on a new identity's key went.
pub(crate) enum PageLock {
    Locked,
    Failed(io::Error),
}

/// A new [`Identity`] and how its page lock went. A lock failure comes
/// alongside a usable identity rather than instead of one: what to do about
/// it (warn and proceed) is the caller's decision.
pub(crate) struct Unlocked {
    pub identity: Identity,
    pub page_lock: PageLock,
}

impl Identity {
    /// Takes ownership of a validated key line and tries once to page-lock
    /// it.
    pub(crate) fn from_key(device: &str, key: KeyLine, locker: Arc<dyn Locker>) -> Unlocked {
        let KeyLine { secret, recipient } = key;
        let (page_lock, page_locked) = match locker.lock(secret.as_slice()) {
            Ok(()) => (PageLock::Locked, true),
            Err(e) => (PageLock::Failed(e), false),
        };
        let identity = Identity {
            device: device.to_string(),
            recipient,
            secret,
            locker,
            page_locked,
        };
        Unlocked {
            identity,
            page_lock,
        }
    }

    /// The device name this identity was unlocked as, within its vault.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// The public key matching this identity's private key: the `age1…`
    /// string in `.age-recipients` and `.gage/config.toml`.
    pub fn recipient(&self) -> &str {
        &self.recipient
    }

    /// Whether the key material is actually pinned in memory. False is a
    /// supported state, not a failure: see `Vault::unlock`'s
    /// warn-and-proceed path.
    pub fn page_locked(&self) -> bool {
        self.page_locked
    }

    /// The key age decrypts with, parsed afresh for one operation.
    pub(crate) fn x25519(&self) -> age::x25519::Identity {
        parse(&self.secret).expect("validated when the Identity was built")
    }

    /// An identity over a bare key, unlocked and never page-locked, for
    /// tests of the crypto wrapper that have no vault or file to unlock
    /// from.
    #[cfg(test)]
    pub(crate) fn for_tests(key: &age::x25519::Identity) -> Identity {
        use age::secrecy::ExposeSecret;
        let secret = LockedBuf::copy_from(key.to_string().expose_secret().as_bytes());
        Identity {
            device: "test-device".into(),
            recipient: key.to_public().to_string(),
            secret,
            locker: Arc::new(crate::crypto::locker::MemLocker),
            page_locked: false,
        }
    }
}

fn parse(secret: &LockedBuf) -> Result<age::x25519::Identity, InvalidKey> {
    let line = std::str::from_utf8(secret.as_slice()).map_err(|_| InvalidKey("not UTF-8"))?;
    line.parse().map_err(InvalidKey)
}

impl Drop for Identity {
    /// Wipe, then unlock, then free (the `LockedBuf` frees its pages when
    /// it drops after this). Zeroing comes first and happens regardless:
    /// leaving key material in memory because an unlock failed would be
    /// worse than the failure. Only a lock that took is released, since
    /// unlocking a page that was never locked is an error on some
    /// platforms. A failed unlock has nowhere to be reported from a
    /// destructor and changes nothing about what the process should do.
    fn drop(&mut self) {
        self.secret.as_mut_slice().zeroize();
        if self.page_locked {
            let _ = self.locker.unlock(self.secret.as_slice());
            self.page_locked = false;
        }
    }
}

// Never print the key, not even in a test failure.
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("device", &self.device)
            .field("recipient", &self.recipient)
            .field("page_locked", &self.page_locked)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    struct Refuses;

    impl Locker for Refuses {
        fn lock(&self, _: &[u8]) -> io::Result<()> {
            Err(io::Error::other("no mlock here"))
        }
        fn unlock(&self, _: &[u8]) -> io::Result<()> {
            panic!("a lock that never took must not be released");
        }
    }

    fn key_line() -> KeyLine {
        let key = age::x25519::Identity::generate();
        KeyLine::parse(key.to_string().expose_secret().as_bytes()).unwrap()
    }

    #[test]
    fn new_reports_a_taken_lock() {
        let u = Identity::from_key("d", key_line(), Arc::new(crate::crypto::locker::MemLocker));
        match u.page_lock {
            // Hosts that refuse mlock fail here; the identity is still usable.
            PageLock::Locked => assert!(u.identity.page_locked()),
            PageLock::Failed(_) => assert!(!u.identity.page_locked()),
        }
    }

    #[test]
    fn new_reports_a_refused_lock_with_a_usable_identity() {
        let u = Identity::from_key("d", key_line(), Arc::new(Refuses));
        assert!(matches!(&u.page_lock, PageLock::Failed(e) if e.to_string() == "no mlock here"));
        assert!(!u.identity.page_locked());
        assert_eq!(u.identity.device(), "d");
        // Dropping it must not call `unlock`.
    }
}
