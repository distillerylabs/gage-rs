//! UniFFI's binding generator, built from the same UniFFI version as the
//! library: `cargo run -p gage-ffi --features bindgen --bin uniffi-bindgen
//! -- generate --library <lib> --language swift --out-dir <dir>`.

fn main() {
    uniffi::uniffi_bindgen_main()
}
