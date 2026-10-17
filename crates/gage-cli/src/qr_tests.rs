//! `-q/--qr`: the renderer, and `show`/`generate` drawing a code instead
//! of printing the value. "The code encodes exactly this value" is checked
//! by turning the render back into its module grid and comparing it with
//! the crate's own grid for the same value, as Go's tests compare with
//! `rsc.io/qr`'s. That catches encoding the wrong thing (the whole entry
//! when `--field` asked for one key) and leaves "does the crate encode
//! correctly" to the crate. No decoder dependency (Q-QR-CRATE).

use gage::exitcode::Code;
use qrcode::{Color, QrCode};

use crate::entry_tests::env_with_vault;
use crate::qr::{QR_BOTH, QR_BOTTOM, QR_LEVEL, QR_NEITHER, QR_QUIET_ZONE, QR_TOP, render_qr};

/// The rendered code's lines out of a command's stdout. `generate -q`
/// prints its confirmation first, which names the entry, not the value.
/// Keeping only lines built from the four glyphs points the grid at the
/// symbol either way, and a value leaked onto a line of its own would
/// still fail the comparison.
pub(crate) fn qr_art_lines(art: &str) -> Vec<&str> {
    art.lines()
        .filter(|l| {
            !l.is_empty()
                && l.chars()
                    .all(|c| [QR_BOTH, QR_TOP, QR_BOTTOM, QR_NEITHER].contains(&c))
        })
        .collect()
}

/// The render turned back into its dark-module grid, quiet zone stripped.
pub(crate) fn decode_rendered_qr(art: &str) -> Vec<Vec<bool>> {
    let lines = qr_art_lines(art);
    assert!(!lines.is_empty(), "no QR output to decode:\n{art}");
    // Each line carries two module rows; `true` is light here.
    let mut rows: Vec<Vec<bool>> = Vec::new();
    for line in lines {
        let (mut top, mut bottom) = (Vec::new(), Vec::new());
        for c in line.chars() {
            let (t, b) = match c {
                QR_BOTH => (true, true),
                QR_TOP => (true, false),
                QR_BOTTOM => (false, true),
                QR_NEITHER => (false, false),
                other => panic!("unexpected glyph {other:?} in QR output"),
            };
            top.push(t);
            bottom.push(b);
        }
        rows.push(top);
        rows.push(bottom);
    }
    let width = rows[0].len();
    let size = width
        .checked_sub(2 * QR_QUIET_ZONE)
        .filter(|&s| s > 0)
        .unwrap_or_else(|| panic!("QR render is {width} columns wide, too narrow for a symbol"));
    (0..size)
        .map(|y| {
            (0..size)
                // Dark modules, the inverse of the render.
                .map(|x| !rows[y + QR_QUIET_ZONE][x + QR_QUIET_ZONE])
                .collect()
        })
        .collect()
}

/// `value` encoded independently: its dark-module grid.
pub(crate) fn reference_grid(value: &str) -> Vec<Vec<bool>> {
    let code = QrCode::with_error_correction_level(value.as_bytes(), QR_LEVEL).unwrap();
    let size = code.width();
    let colors = code.to_colors();
    (0..size)
        .map(|y| {
            (0..size)
                .map(|x| colors[y * size + x] == Color::Dark)
                .collect()
        })
        .collect()
}

fn render_lines(value: &str) -> Vec<String> {
    render_qr(value)
        .unwrap()
        .trim_end_matches('\n')
        .split('\n')
        .map(str::to_string)
        .collect()
}

// Go: TestRenderQRRoundTripsToTheExactValue. No truncation, no trailing
// newline swept in, no padding that shifts the grid.
#[test]
fn render_qr_round_trips_to_the_exact_value() {
    for value in [
        "hunter2",
        "JBSWY3DPEHPK3PXP",
        "correct horse battery staple",
        "https://example.com/totp?secret=ABC123&issuer=gage",
    ] {
        let art = render_qr(value).unwrap();
        assert_eq!(
            decode_rendered_qr(&art),
            reference_grid(value),
            "the render for {value:?} is not that value's symbol"
        );
    }
}

// Go: TestRenderQRDistinguishesValues. What gives the round trip its
// teeth: a comparison that matched everything would prove nothing.
#[test]
fn render_qr_distinguishes_values() {
    let art = render_qr("hunter2").unwrap();
    assert_ne!(decode_rendered_qr(&art), reference_grid("hunter3"));
}

// Go: TestRenderQRHasAQuietZone. The top QR_QUIET_ZONE/2 lines are solid
// border, and so are the first and last QR_QUIET_ZONE columns of every
// line.
#[test]
fn render_qr_has_a_quiet_zone() {
    let lines = render_lines("hunter2");
    for line in &lines[..QR_QUIET_ZONE / 2] {
        assert!(
            line.chars().all(|c| c == QR_BOTH),
            "not solid quiet zone: {line:?}"
        );
    }
    // And every line wholly below the symbol, which with an odd module
    // count includes the padding row (see `render_qr`).
    let size = reference_grid("hunter2").len();
    let symbol_lines = (QR_QUIET_ZONE + size).div_ceil(2);
    for line in &lines[symbol_lines..] {
        assert!(
            line.chars().all(|c| c == QR_BOTH),
            "not solid quiet zone: {line:?}"
        );
    }
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for x in 0..QR_QUIET_ZONE {
            assert!(
                chars[x] == QR_BOTH && chars[chars.len() - 1 - x] == QR_BOTH,
                "line {i} lacks a quiet-zone margin: {line:?}"
            );
        }
    }
}

// Go: TestRenderQRIsSquareIshAndRectangular. Every line the same width, and
// two module rows per line.
#[test]
fn render_qr_is_square_ish_and_rectangular() {
    let lines = render_lines("hunter2");
    let width = lines[0].chars().count();
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(line.chars().count(), width, "line {i}");
    }
    assert_eq!(lines.len(), width.div_ceil(2));
}

// Go: TestRenderQRRejectsAnEmptyValue. Nothing to scan, and an empty symbol
// would be a silently useless answer.
#[test]
fn render_qr_rejects_an_empty_value() {
    let err = render_qr("").unwrap_err();
    assert_eq!(err.code(), Code::Usage);
}

// Beyond Go's rows: a value no QR version holds is a usage error naming the
// cause, not a crash or a partial symbol.
#[test]
fn render_qr_rejects_a_value_too_long_to_encode() {
    let err = render_qr(&"x".repeat(8000)).unwrap_err();
    assert_eq!(err.code(), Code::Usage);
    assert!(err.to_string().contains("too long"), "{err}");
}

// --- show / generate -q ---------------------------------------------------------

// Go: TestShowQRPrintsNoPlaintextAlongsideTheCode. -q renders *instead of*
// printing: a code with the secret written under it defeats itself.
#[test]
fn show_qr_prints_no_plaintext_alongside_the_code() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["GitHub"]);
    let r = env.cmd_ok(&["show", "GitHub", "-q"]);
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
    assert!(!r.stderr.contains("hunter2"), "{}", r.stderr);
    // Nothing but the code on stdout.
    assert_eq!(qr_art_lines(&r.stdout).len(), r.stdout.lines().count());
    assert_eq!(decode_rendered_qr(&r.stdout), reference_grid("hunter2"));
}

// Go: TestShowFieldQREncodesOnlyThatField. A structured note must not be
// dumped whole into a code when only the TOTP seed was wanted.
#[test]
fn show_field_qr_encodes_only_that_field() {
    let env = env_with_vault();
    env.insert_ok(
        "the-password",
        &[
            "Proton",
            "--field",
            "username=me@example.com",
            "--field",
            "totp_seed=JBSWY3DPEHPK3PXP",
        ],
    );
    let r = env.cmd_ok(&["show", "Proton", "--field", "totp_seed", "-q"]);
    assert_eq!(
        decode_rendered_qr(&r.stdout),
        reference_grid("JBSWY3DPEHPK3PXP")
    );
    for leak in ["the-password", "me@example.com", "JBSWY3DPEHPK3PXP"] {
        assert!(!r.stdout.contains(leak), "QR output carries {leak:?}");
    }
}

// Go (generate): TestGenerateQRRendersWithoutPrintingTheValue. The code is
// the value that was stored, and the value itself is printed nowhere.
#[test]
fn generate_qr_renders_without_printing_the_value() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["generate", "GitHub", "-q"]);
    let stored = env.only_entry("personal").1.value;
    assert!(!r.stdout.contains(&stored), "{}", r.stdout);
    assert!(!r.stderr.contains(&stored), "{}", r.stderr);
    assert!(r.stdout.contains("generated"), "{}", r.stdout);
    assert_eq!(decode_rendered_qr(&r.stdout), reference_grid(&stored));
}

// An empty value is a usage error through the command too, not an empty
// symbol.
#[test]
fn show_qr_of_an_empty_field_is_a_usage_error() {
    let env = env_with_vault();
    env.insert_ok("v", &["Proton", "--field", "note="]);
    let r = env.cmd(&["show", "Proton", "--field", "note", "-q"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stdout.is_empty(), "{}", r.stdout);
}
