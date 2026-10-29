//! The vault type and the unlock method: the two closed vocabularies the
//! config files name, as enums rather than strings.
//!
//! They differ on purpose (ADR-0053). An unknown type is refused as the
//! file is read: v2 has one type, and a vault this build can't operate on
//! must not be half-handled as if it were git. An unknown method is kept:
//! a method a later build adds must not make this one refuse the whole
//! vault config, so it parses, round-trips byte for byte, and is refused
//! only where it would be used, at unlock and wherever one is chosen.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, HasKind};

/// The kind of store a vault is. Today's only one is git.
///
/// Listed in [`VaultType::ALL`] rather than hardcoded, so a second type is
/// additive to the CLI surface rather than a breaking change ("Vault
/// types", ADR-0012).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultType {
    /// `Default` because a global-config row is built from defaults; the
    /// committed config requires the key.
    #[default]
    Git,
}

/// A vault type this build doesn't know, as given.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown vault type {0:?}; accepted: {accepted}", accepted = VaultType::names(", "))]
pub struct UnknownVaultType(pub String);

impl HasKind for UnknownVaultType {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Usage
    }
}

impl VaultType {
    /// Every type this build accepts.
    pub const ALL: &[VaultType] = &[VaultType::Git];

    pub fn as_str(&self) -> &'static str {
        match self {
            VaultType::Git => "git",
        }
    }

    /// Every accepted type's name, joined by `sep`: what help and a
    /// refusal list.
    pub fn names(sep: &str) -> String {
        names(VaultType::ALL, |t| t.as_str(), sep)
    }
}

impl FromStr for VaultType {
    type Err = UnknownVaultType;

    fn from_str(s: &str) -> Result<VaultType, UnknownVaultType> {
        VaultType::ALL
            .iter()
            .find(|t| t.as_str() == s)
            .copied()
            .ok_or_else(|| UnknownVaultType(s.to_string()))
    }
}

impl fmt::Display for VaultType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for VaultType {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// How a device's identity file is unlocked. Today's only performed method
/// is a passphrase; anything else read from a file is kept as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum UnlockMethod {
    Passphrase,
    /// A method this build doesn't perform, spelled as the file spelled it.
    /// The empty string is one: it is what an absent key reads as.
    Other(String),
}

impl UnlockMethod {
    /// Every method this build performs.
    pub const SUPPORTED: &[UnlockMethod] = &[UnlockMethod::Passphrase];

    pub fn as_str(&self) -> &str {
        match self {
            UnlockMethod::Passphrase => "passphrase",
            UnlockMethod::Other(s) => s,
        }
    }

    /// Whether this build performs it.
    pub fn is_supported(&self) -> bool {
        !matches!(self, UnlockMethod::Other(_))
    }

    /// Every performed method's name, joined by `sep`: what help and a
    /// refusal list.
    pub fn names(sep: &str) -> String {
        names(UnlockMethod::SUPPORTED, UnlockMethod::as_str, sep)
    }
}

/// An absent method reads as the empty string it always did, which no
/// build performs: never as a passphrase nobody chose.
impl Default for UnlockMethod {
    fn default() -> UnlockMethod {
        UnlockMethod::Other(String::new())
    }
}

impl From<String> for UnlockMethod {
    fn from(s: String) -> UnlockMethod {
        match s.as_str() {
            "passphrase" => UnlockMethod::Passphrase,
            _ => UnlockMethod::Other(s),
        }
    }
}

impl From<&str> for UnlockMethod {
    fn from(s: &str) -> UnlockMethod {
        UnlockMethod::from(s.to_string())
    }
}

impl From<UnlockMethod> for String {
    fn from(m: UnlockMethod) -> String {
        match m {
            UnlockMethod::Other(s) => s,
            known => known.as_str().to_string(),
        }
    }
}

impl fmt::Display for UnlockMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for UnlockMethod {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

fn names<T>(all: &[T], name: impl Fn(&T) -> &str, sep: &str) -> String {
    all.iter().map(name).collect::<Vec<_>>().join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(rename = "type")]
        vault_type: VaultType,
        method: UnlockMethod,
    }

    fn parse(text: &str) -> Result<Holder, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn git_and_passphrase_parse_to_their_variants() {
        let h = parse("type = \"git\"\nmethod = \"passphrase\"\n").unwrap();
        assert_eq!(h.vault_type, VaultType::Git);
        assert_eq!(h.method, UnlockMethod::Passphrase);
    }

    // Strict: a type this build can't operate on is refused at parse, and
    // the refusal names it.
    #[test]
    fn an_unknown_type_is_refused_naming_the_value() {
        let err = parse("type = \"s3\"\nmethod = \"passphrase\"\n").unwrap_err();
        assert!(err.to_string().contains("s3"), "{err}");
        let err = "s3".parse::<VaultType>().unwrap_err();
        assert_eq!(err, UnknownVaultType("s3".into()));
        assert_eq!(err.to_string(), "unknown vault type \"s3\"; accepted: git");
        assert_eq!(err.kind(), ErrorKind::Usage);
    }

    // Lenient: a method a later build adds parses, and is written back
    // exactly as it was read.
    #[test]
    fn an_unknown_method_is_kept_and_reserializes_byte_identically() {
        for method in ["yubikey", "plugin:foo", ""] {
            let text = format!("type = \"git\"\nmethod = \"{method}\"\n");
            let h = parse(&text).unwrap();
            assert_eq!(h.method, UnlockMethod::Other(method.into()));
            assert!(!h.method.is_supported());
            assert_eq!(toml::to_string(&h).unwrap(), text);
        }
    }

    #[test]
    fn names_and_lists_come_from_the_enums() {
        assert_eq!(VaultType::Git.to_string(), "git");
        assert_eq!("git".parse::<VaultType>().unwrap(), VaultType::Git);
        assert_eq!(UnlockMethod::Passphrase.to_string(), "passphrase");
        assert_eq!(UnlockMethod::from("passphrase"), UnlockMethod::Passphrase);
        assert!(UnlockMethod::Passphrase.is_supported());
        assert_eq!(VaultType::names(" "), "git");
        assert_eq!(UnlockMethod::names(" "), "passphrase");
        assert_eq!(UnlockMethod::default(), UnlockMethod::Other(String::new()));
    }
}
