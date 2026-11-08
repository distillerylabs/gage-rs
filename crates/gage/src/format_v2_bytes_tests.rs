//! Byte-exact re-serialization of the TOML schemas in `doc/format/v2.md`,
//! where it doesn't need v2's vault opened: format v3 kept every schema
//! unchanged, but this build refuses a v2 vault (`format_v2_tests`), so the
//! committed vault config and trust record are re-serialized in
//! `format_v3_bytes_tests` instead.
//!
//! These compare *bytes*: each committed file is read the way gage reads
//! it and written the way gage writes it, and must come back identical.
//! They were written green against the `String`-typed schemas, before D3a
//! re-typed them.

use std::path::Path;

use crate::crypto::crypt;
use crate::enrollment::{Payload, normalize_code};
use crate::storage::config;
use crate::storage::vaultconfig;
use crate::testsupport::format_v2 as fx;
use crate::trust::trustcache::TrustRecord;

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

// The template, not the installed copy: install fills in temp paths, and a
// Windows path's backslashes would let the serializer pick a different
// string style for a reason that has nothing to do with the schema.
#[test]
fn format_v2_global_config_reserializes_byte_identically() {
    let template = text(&fx::fixture_dir().join("config").join("config.toml.in"));
    let committed = template
        .replace(fx::VAULT_PATH_PLACEHOLDER, "/data/gage/vaults/personal")
        .replace(fx::ORIGIN_PLACEHOLDER, "https://example.com/personal.git");
    assert_ne!(committed, template);

    let dir = tempfile::tempdir().unwrap();
    let (src, dst) = (dir.path().join("in.toml"), dir.path().join("out.toml"));
    std::fs::write(&src, &committed).unwrap();
    config::save(&dst, &config::load(&src).unwrap()).unwrap();
    assert_eq!(text(&dst), committed);
}

#[test]
fn format_v2_enrollment_payload_reserializes_byte_identically() {
    let i = fx::install();
    let pending = i.vault.join(".gage/pending");
    let mut names: Vec<_> = std::fs::read_dir(&pending)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    let sealed = std::fs::read(names.pop().unwrap()).unwrap();
    let code = normalize_code(fx::ENROLLMENT_CODE).unwrap();
    let plaintext =
        crypt::decrypt_with(&sealed, &crypt::enrollment_scrypt_identity(&code)).unwrap();
    let plaintext = String::from_utf8(plaintext).unwrap();

    // What seal_payload serializes with.
    let payload: Payload = toml::from_str(&plaintext).unwrap();
    assert_eq!(toml::to_string(&payload).unwrap(), plaintext);
}

/// `text` parsed as `T` and serialized again.
fn reserialize<T>(text: &str) -> String
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    toml::to_string(&toml::from_str::<T>(text).unwrap()).unwrap()
}

// The optional keys a writer omits when empty (v2 §2.2, §3.2, §3.4),
// spelled as text so the test outlives a change of field type: `key = ""`
// in a file must still mean what an absent key means.
#[test]
fn format_v2_empty_optional_keys_are_omitted() {
    let global = reserialize::<config::Global>(
        "current = \"\"\n\
         [vaults.v]\n\
         path = \"/p\"\n\
         id = \"\"\n\
         type = \"git\"\n\
         device = \"laptop-1\"\n\
         pubkey = \"\"\n\
         method = \"passphrase\"\n\
         [vaults.v.git]\n\
         origin = \"\"\n\
         [shell]\n\
         prompt = \"\"\n\
         idle_timeout = \"\"\n\
         history_file = \"\"\n\
         clipboard_timeout = \"\"\n",
    );
    assert_eq!(
        global,
        "[vaults.v]\n\
         path = \"/p\"\n\
         type = \"git\"\n\
         device = \"laptop-1\"\n\
         method = \"passphrase\"\n"
    );

    let file = reserialize::<vaultconfig::File>(&format!(
        "[vault]\n\
         name = \"v\"\n\
         id = \"{}\"\n\
         type = \"git\"\n\
         format_version = 2\n\
         created = \"2026-10-28\"\n\
         [method]\n\
         default = \"passphrase\"\n\
         plugin = \"\"\n",
        fx::VAULT_ID
    ));
    assert!(!file.contains("plugin"), "{file}");
    assert!(
        file.contains("[method]\ndefault = \"passphrase\"\n"),
        "{file}"
    );

    let record = reserialize::<TrustRecord>("recipients_hash = \"ab\"\nmismatch_seen = \"\"\n");
    assert_eq!(record, "recipients_hash = \"ab\"\n");
}

// A required key is written even when empty: omitting it would change
// what an older reader sees.
#[test]
fn format_v2_empty_required_keys_are_written() {
    let entry = "[vaults.v]\n\
                 path = \"\"\n\
                 type = \"git\"\n\
                 device = \"\"\n\
                 method = \"passphrase\"\n";
    assert_eq!(reserialize::<config::Global>(entry), entry);

    let file = format!(
        "[vault]\n\
         name = \"\"\n\
         id = \"{}\"\n\
         type = \"git\"\n\
         format_version = 2\n\
         created = \"\"\n\
         \n\
         [method]\n\
         default = \"passphrase\"\n\
         \n\
         [[recipients]]\n\
         device = \"laptop-1\"\n\
         pubkey = \"{}\"\n",
        fx::VAULT_ID,
        fx::DEVICE_PUBKEY
    );
    assert_eq!(reserialize::<vaultconfig::File>(&file), file);

    let record = "recipients_hash = \"\"\n";
    assert_eq!(reserialize::<TrustRecord>(record), record);
}

// D3a's `Option` rule, read side: every converted field reads `key = ""`
// and an absent key alike, as `None`. The write side is
// `format_v2_empty_optional_keys_are_omitted`.
#[test]
fn format_v2_empty_and_absent_optional_keys_read_as_none() {
    let entry =
        "[vaults.v]\npath = \"/p\"\ntype = \"git\"\ndevice = \"d\"\nmethod = \"passphrase\"\n";
    let empty = format!(
        "current = \"\"\n{entry}id = \"\"\npubkey = \"\"\n[vaults.v.git]\norigin = \"\"\n\
         [shell]\nprompt = \"\"\nidle_timeout = \"\"\nhistory_file = \"\"\nclipboard_timeout = \"\"\n"
    );
    for text in [entry.to_string(), empty] {
        let g: config::Global = toml::from_str(&text).unwrap();
        assert_eq!(g.current, None, "{text}");
        let v = &g.vaults["v"];
        assert_eq!(
            (&v.id, &v.pubkey, &v.git.origin),
            (&None, &None, &None),
            "{text}"
        );
        assert_eq!(g.shell, config::Shell::default(), "{text}");
    }

    let config = |extra: &str| {
        format!(
            "[vault]\nname = \"v\"\nid = \"{}\"\ntype = \"git\"\nformat_version = 2\n\
             created = \"2026-10-28\"\n[method]\ndefault = \"passphrase\"\n{extra}",
            fx::VAULT_ID
        )
    };
    for text in [config(""), config("plugin = \"\"\n")] {
        let file: vaultconfig::File = toml::from_str(&text).unwrap();
        assert_eq!(file.method.plugin, None, "{text}");
    }

    for text in [
        "recipients_hash = \"ab\"\n",
        "recipients_hash = \"ab\"\nmismatch_seen = \"\"\n",
    ] {
        let record: TrustRecord = toml::from_str(text).unwrap();
        assert_eq!(record.mismatch_seen, None, "{text}");
    }
}
