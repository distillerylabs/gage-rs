//! `gage cat`'s display pass over an entry's YAML: the output is still the
//! entry's full plaintext for scripting and piping, with three changes
//! that exist only on screen.
//!
//! - an `id: <short id>` line right after `title`, which is not part of
//!   the stored entry
//! - top-level keys aligned in one column, and the `fields` mapping's keys
//!   aligned independently of them
//! - multi-line block values printed flush left, as the value's own lines
//!
//! Which lines are keys, and where a block value ends, comes from the
//! library's parse of the document ([`gage::entry::key_layout`]), never
//! from how a line looks: a secret whose text reads `note: x` is content.
//! The result is display, not a serialization: flush-left block content is
//! no longer YAML at that indentation, so a value that spans lines doesn't
//! parse back.

use gage::entry::{KeyLine, key_layout};
use zeroize::Zeroizing;

/// `data` with the id line, aligned keys and dedented blocks. Anything that
/// isn't a mapping of scalar keys is returned unchanged.
pub fn align_cat_output(short_id: &str, data: &[u8]) -> Vec<u8> {
    let Some(layout) = key_layout(data) else {
        return data.to_vec();
    };
    let Ok(text) = std::str::from_utf8(data) else {
        return data.to_vec();
    };
    let trailing_newline = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines: Vec<String> = body.split('\n').map(String::from).collect();
    let line_count = lines.len();

    // The id line goes where the first key's value ends: right after
    // `title`, which `marshal` always writes first.
    let id_at = layout
        .first()
        .and_then(|first| first.end)
        .unwrap_or(line_count)
        .min(line_count);

    let mut spans: Vec<(usize, usize, Zeroizing<String>)> = Vec::new();
    let mut top: Vec<(usize, &str)> = Vec::new();
    for key in &layout {
        if key.plain {
            top.push((key.line, key.key.as_str()));
        }
        if !key.fields.is_empty() {
            let sub: Vec<(usize, &str)> = key
                .fields
                .iter()
                .filter(|f| f.plain)
                .map(|f| (f.line, f.key.as_str()))
                .collect();
            pad_key_lines(&mut lines, &sub);
            for f in &key.fields {
                push_span(&mut spans, f, line_count);
            }
        }
        push_span(&mut spans, key, line_count);
    }
    spans.sort_by_key(|s| s.0);

    // `id` is a top-level key like the rest, so it counts toward the width.
    let width = top
        .iter()
        .map(|(_, k)| k.chars().count())
        .chain(std::iter::once("id".len()))
        .max()
        .unwrap_or(0);
    pad_to(&mut lines, &top, width);
    let id_line = format!("id{}: {short_id}", " ".repeat(width - "id".len()));

    let mut out: Vec<String> = Vec::with_capacity(line_count + 1);
    let mut spans = spans.into_iter().peekable();
    let mut i = 0;
    while i < line_count {
        if i == id_at {
            out.push(id_line.clone());
        }
        if let Some((start, end, content)) = spans.next_if(|s| s.0 == i) {
            debug_assert!(start == i);
            out.extend(content.split('\n').map(String::from));
            i = end;
            continue;
        }
        out.push(std::mem::take(&mut lines[i]));
        i += 1;
    }
    if id_at >= line_count {
        out.push(id_line);
    }

    let mut result = out.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result.into_bytes()
}

/// A multi-line block value's content lines are replaced by the value's
/// own lines, flush left.
fn push_span(spans: &mut Vec<(usize, usize, Zeroizing<String>)>, key: &KeyLine, line_count: usize) {
    if let Some(content) = &key.literal {
        let end = key.end.unwrap_or(line_count).min(line_count);
        if key.line + 1 < end {
            spans.push((key.line + 1, end, content.clone()));
        }
    }
}

/// Pads each listed key, in place, to the width of the widest one so every
/// colon lands in one column. `keys` pairs a line index with the key's
/// text. A line that doesn't hold its key where the parse says it should
/// is left alone rather than risk corrupting it.
pub fn pad_key_lines(lines: &mut [String], keys: &[(usize, &str)]) {
    if let Some(width) = keys.iter().map(|(_, k)| k.chars().count()).max() {
        pad_to(lines, keys, width);
    }
}

fn pad_to(lines: &mut [String], keys: &[(usize, &str)], width: usize) {
    for &(idx, key) in keys {
        let Some(line) = lines.get_mut(idx) else {
            continue;
        };
        let indent = line.len() - line.trim_start_matches(' ').len();
        if !line[indent..].starts_with(key) {
            continue;
        }
        let pad = " ".repeat(width.saturating_sub(key.chars().count()));
        line.insert_str(indent + key.len(), &pad);
    }
}
