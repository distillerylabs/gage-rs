//! The library's top-level layout (D4a, ADR-0055): `lib.rs` declares these
//! public modules and no others. A new area is a deliberate change to this
//! list, not something that drifts in.

const PUBLIC_MODULES: &[&str] = &[
    "crypto",
    "devicename",
    "enrollment",
    "entry",
    "error",
    "prompter",
    "recovery",
    "session",
    "storage",
    "sync",
    "trust",
    "vault",
];

/// Test-gated modules, which other crates' tests reach through the
/// `test-support` feature.
const TEST_SUPPORT_MODULES: &[&str] = &["gittest", "testsupport"];

fn top_level_pub_mods() -> Vec<String> {
    let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    assert!(
        text.contains("#![deny(clippy::disallowed_macros"),
        "the scan is not reading lib.rs"
    );
    let mut mods: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("pub mod "))
        .map(|l| l.trim_end_matches(';').to_string())
        .collect();
    mods.sort();
    mods
}

#[test]
fn lib_rs_declares_exactly_the_areas() {
    let mut want: Vec<String> = PUBLIC_MODULES
        .iter()
        .chain(TEST_SUPPORT_MODULES)
        .map(|s| s.to_string())
        .collect();
    want.sort();
    assert_eq!(top_level_pub_mods(), want);
}

#[test]
fn no_recipients_module_remains() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(src.join("lib.rs").exists());
    assert!(!src.join("recipients.rs").exists());
}
