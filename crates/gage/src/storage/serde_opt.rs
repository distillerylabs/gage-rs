//! Reading an optional TOML key whose empty string means absent.
//!
//! Before D3 these fields were `String`s skipped on write when empty, so a
//! file gage wrote never holds `key = ""`, but an older or hand-edited one
//! may, and it must mean what it meant then: nothing. Writing is plain
//! `skip_serializing_if = "Option::is_none"`.

use std::fmt::Display;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// `key = ""` and an absent key read as `None`; anything else goes
/// through `T::try_from`, so a typed field refuses a bad value, and the
/// empty check runs first so `""` is never offered to it.
///
/// An absent key needs `#[serde(default)]` on the field or its container.
pub(crate) fn empty_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<String>,
    T::Error: Display,
{
    match Option::<String>::deserialize(deserializer)? {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => T::try_from(s).map(Some).map_err(D::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use crate::storage::vaultconfig::VaultId;

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct Holder {
        #[serde(deserialize_with = "super::empty_as_none")]
        text: Option<String>,
        #[serde(deserialize_with = "super::empty_as_none")]
        id: Option<VaultId>,
    }

    fn parse(text: &str) -> Result<Holder, toml::de::Error> {
        toml::from_str(text)
    }

    #[test]
    fn empty_and_absent_read_as_none() {
        for text in ["", "text = \"\"\nid = \"\"\n"] {
            let h = parse(text).unwrap();
            assert_eq!(h.text, None, "{text:?}");
            assert_eq!(h.id, None, "{text:?}");
        }
    }

    #[test]
    fn a_value_reads_as_some() {
        let h = parse("text = \"x\"\nid = \"9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497\"\n").unwrap();
        assert_eq!(h.text.as_deref(), Some("x"));
        assert_eq!(h.id.unwrap(), "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497");
    }

    // Only "" means absent: a typed field still refuses a bad value.
    #[test]
    fn a_bad_typed_value_is_refused_not_dropped() {
        let err = parse("id = \"x\"\n").unwrap_err();
        assert!(
            err.to_string().contains("vault id \"x\" is not a valid id"),
            "{err}"
        );
    }
}
