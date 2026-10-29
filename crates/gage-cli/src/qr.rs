//! `-q/--qr`: a value as a terminal QR code (ADR-0049). The crate does
//! the encoding and gage does the drawing; see "Terminal ASCII QR
//! rendering" in the design doc, which names that split and notes that a
//! GUI would draw the same modules as an image instead.

use gage::exitcode::Code;
use qrcode::{Color, EcLevel, QrCode};
use zeroize::Zeroizing;

use crate::error::CliError;

/// The error-correction level every gage QR is encoded at. Medium (~15%
/// recoverable) rather than Low: what's being scanned is a terminal
/// window, where a partly obscured or reflowed render is routine. Fixed,
/// since a scanner reads the level out of the symbol.
pub const QR_LEVEL: EcLevel = EcLevel::M;

/// The light border around the symbol, in modules. Four is the spec's
/// minimum; without it a scanner can't find the symbol's edge against
/// whatever the terminal shows behind it.
pub const QR_QUIET_ZONE: usize = 4;

// Half blocks: each character carries two vertically stacked modules, which
// keeps the code roughly square in a terminal cell about twice as tall as
// it is wide.
/// Both modules light.
pub const QR_BOTH: char = '█';
/// Top module light.
pub const QR_TOP: char = '▀';
/// Bottom module light.
pub const QR_BOTTOM: char = '▄';
/// Both modules dark.
pub const QR_NEITHER: char = ' ';

/// `value` as a terminal QR code, one line per two module rows.
///
/// Light modules are glyphs and dark ones spaces, the inverse of ink on
/// paper: a terminal draws glyphs in the foreground colour over a darker
/// background, so this is what puts light where the symbol needs it. The
/// render encodes the value, so it is as secret as the value and is
/// zeroized on drop like it.
pub fn render_qr(value: &str) -> Result<Zeroizing<String>, CliError> {
    if value.is_empty() {
        return Err(CliError::new(
            Code::Usage,
            "gage: nothing to encode as a QR code: the value is empty",
        ));
    }
    let code = QrCode::with_error_correction_level(value.as_bytes(), QR_LEVEL).map_err(|e| {
        // Realistically a value too large for any QR version (a long note,
        // an SSH key), where the answer is another output mode.
        CliError::new(
            Code::Usage,
            format!("gage: this value cannot be encoded as a QR code (too long?): {e}"),
        )
    })?;
    let size = code.width();
    let colors = code.to_colors();

    // The one place the quiet-zone offset is applied, so the border and
    // the symbol can't disagree about where the symbol starts.
    let span = size + 2 * QR_QUIET_ZONE;
    let light = |x: usize, y: usize| {
        let inside = QR_QUIET_ZONE..QR_QUIET_ZONE + size;
        if !inside.contains(&x) || !inside.contains(&y) {
            return true;
        }
        colors[(y - QR_QUIET_ZONE) * size + (x - QR_QUIET_ZONE)] == Color::Light
    };

    let mut out = Zeroizing::new(String::with_capacity((span + 1) * span.div_ceil(2) * 3));
    for y in (0..span).step_by(2) {
        for x in 0..span {
            // An odd row count leaves the last line no bottom module. It
            // reads as quiet zone, extending the border by one module
            // rather than truncating the symbol.
            let top = light(x, y);
            let bottom = y + 1 >= span || light(x, y + 1);
            out.push(match (top, bottom) {
                (true, true) => QR_BOTH,
                (true, false) => QR_TOP,
                (false, true) => QR_BOTTOM,
                (false, false) => QR_NEITHER,
            });
        }
        out.push('\n');
    }
    Ok(out)
}
