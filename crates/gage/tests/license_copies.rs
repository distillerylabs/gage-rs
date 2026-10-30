//! Each published crate ships its own copy of the root `LICENSE` (cargo
//! packages only files inside the crate directory). The copies must stay
//! byte-identical to the root file.

use std::fs;

#[test]
fn published_crates_carry_the_root_license_verbatim() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let canonical = fs::read(format!("{root}/LICENSE")).expect("root LICENSE");
    for krate in ["gage", "gage-cli"] {
        let copy = fs::read(format!("{root}/crates/{krate}/LICENSE"))
            .unwrap_or_else(|e| panic!("crates/{krate}/LICENSE: {e}"));
        assert!(
            copy == canonical,
            "crates/{krate}/LICENSE differs from the root LICENSE"
        );
    }
}
