//! The recovery keypair. No I/O: generating one touches neither disk nor
//! vault.

use age::secrecy::ExposeSecret;
use zeroize::Zeroizing;

use super::RecoveryError;
use crate::vault::Vault;

/// A freshly generated recovery keypair. `secret` is the one thing a
/// frontend must show the human once; the library never persists it.
pub struct RecoveryKey {
    /// The `age1...` recipient to register.
    pub pubkey: String,
    /// The `AGE-SECRET-KEY-1...` line, zeroed on drop
    /// (ADR-0027).
    pub secret: Zeroizing<String>,
}

// Hand-written so a stray `{:?}` can never print the secret.
impl std::fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryKey")
            .field("pubkey", &self.pubkey)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Generates a new recovery keypair.
pub fn new_recovery_key() -> Result<RecoveryKey, RecoveryError> {
    let identity = age::x25519::Identity::generate();
    // The SecretString is dropped (and zeroed) at the end of this scope;
    // only the Zeroizing copy outlives it.
    let secret = Zeroizing::new(identity.to_string().expose_secret().to_string());
    Ok(RecoveryKey {
        pubkey: identity.to_public().to_string(),
        secret,
    })
}

impl Vault {
    /// Checks a pasted recovery secret against this vault's recipients and
    /// returns the label it is registered under.
    ///
    /// It reads only the plaintext recipient list: no unlock, no decryption,
    /// no lock. A device's key is accepted too and returns that device's
    /// label; whether that is what the human meant is the caller's call.
    ///
    /// Surrounding whitespace is trimmed, as a paste leaves it. This neither
    /// retains nor zeroes `secret`: wiping the caller's buffer is the
    /// caller's job, unlike `recover_device`, which takes ownership of the
    /// secret it uses.
    pub fn verify_recovery_key(&self, secret: &str) -> Result<String, RecoveryError> {
        // The parse error is dropped, not wrapped: it can quote the input.
        let identity: age::x25519::Identity = secret
            .trim()
            .parse()
            .map_err(|_| RecoveryError::MalformedRecoveryKey)?;
        let pubkey = identity.to_public().to_string();
        self.recipients()?
            .into_iter()
            .find(|r| r.pubkey == pubkey)
            .map(|r| r.device)
            .ok_or_else(|| RecoveryError::NotARecipient {
                pubkey,
                vault: self.name.clone(),
            })
    }
}
