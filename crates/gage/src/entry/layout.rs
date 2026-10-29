//! Where each key of an entry's YAML sits in the text, for `gage cat`'s
//! display pass. The library reads the document structure (with
//! `saphyr-parser`'s spans) so the frontend never has to guess a key from
//! its line: a multi-line secret that reads `note: x` is a value's content,
//! not a key, and only the parse knows that.
//!
//! This reports positions and nothing else. Padding, the `id` line and
//! dedenting are the frontend's rendering (ADR-0048).

use saphyr_parser::{Event, Parser, ScalarStyle, Span, SpannedEventReceiver};
use zeroize::Zeroizing;

/// One mapping key and what follows it. Lines are 0-based indexes into the
/// text's lines.
#[derive(Clone)]
pub struct KeyLine {
    pub key: String,
    /// The line the key's text starts on.
    pub line: usize,
    /// Whether the key is written bare. A quoted key is reported (its
    /// siblings' extents depend on it) but a frontend should leave its
    /// text alone.
    pub plain: bool,
    /// The line the next sibling starts on, or else the line after this
    /// key's parent ends, or `None` when this key runs to the end of the
    /// text. Everything from `line + 1` up to here belongs to the value.
    pub end: Option<usize>,
    /// The value, when it is a multi-line `|` block scalar: the one style
    /// whose lines are independently indented content.
    pub literal: Option<Zeroizing<String>>,
    /// The keys of the `fields` mapping, for the top-level `fields` key
    /// only.
    pub fields: Vec<KeyLine>,
}

enum Node {
    Scalar {
        text: String,
        style: ScalarStyle,
        line: usize,
    },
    Map {
        line: usize,
        pairs: Vec<(Node, Node)>,
    },
    Other {
        line: usize,
    },
}

impl Node {
    fn line(&self) -> usize {
        match self {
            Node::Scalar { line, .. } | Node::Map { line, .. } | Node::Other { line } => *line,
        }
    }
}

struct Events<'a>(Vec<(Event<'a>, Span)>);

impl<'a> SpannedEventReceiver<'a> for Events<'a> {
    fn on_event(&mut self, ev: Event<'a>, span: Span) {
        self.0.push((ev, span));
    }
}

/// The top-level keys of `data`'s first YAML document, or `None` when it
/// isn't text, doesn't parse, or isn't a mapping with scalar keys. `None`
/// means "leave this alone".
pub fn key_layout(data: &[u8]) -> Option<Vec<KeyLine>> {
    let text = std::str::from_utf8(data).ok()?;
    let mut events = Events(Vec::new());
    Parser::new_from_str(text).load(&mut events, false).ok()?;
    let mut pos = 0;
    // Skip StreamStart / DocumentStart.
    while let Some((ev, _)) = events.0.get(pos) {
        pos += 1;
        if matches!(ev, Event::DocumentStart(_)) {
            break;
        }
    }
    match node(&events.0, &mut pos)? {
        Node::Map { pairs, .. } => build(&pairs, None, true),
        _ => None,
    }
}

fn node(events: &[(Event<'_>, Span)], pos: &mut usize) -> Option<Node> {
    let (ev, span) = events.get(*pos)?;
    *pos += 1;
    let line = span.start.line().checked_sub(1)?;
    match ev {
        Event::Scalar(text, style, ..) => Some(Node::Scalar {
            text: text.to_string(),
            style: *style,
            line,
        }),
        Event::MappingStart(..) => {
            let mut pairs = Vec::new();
            while !matches!(events.get(*pos)?.0, Event::MappingEnd) {
                let k = node(events, pos)?;
                let v = node(events, pos)?;
                pairs.push((k, v));
            }
            *pos += 1;
            Some(Node::Map { line, pairs })
        }
        Event::SequenceStart(..) => {
            let mut depth = 1;
            while depth > 0 {
                match events.get(*pos)?.0 {
                    Event::SequenceStart(..) | Event::MappingStart(..) => depth += 1,
                    Event::SequenceEnd | Event::MappingEnd => depth -= 1,
                    _ => {}
                }
                *pos += 1;
            }
            Some(Node::Other { line })
        }
        Event::Alias(_) => Some(Node::Other { line }),
        _ => None,
    }
}

fn build(pairs: &[(Node, Node)], parent_end: Option<usize>, top: bool) -> Option<Vec<KeyLine>> {
    let mut out = Vec::with_capacity(pairs.len());
    for (i, (k, v)) in pairs.iter().enumerate() {
        let Node::Scalar { text, style, line } = k else {
            return None;
        };
        let end = pairs.get(i + 1).map(|(next, _)| next.line()).or(parent_end);
        let literal = match v {
            Node::Scalar {
                text,
                style: ScalarStyle::Literal,
                ..
            } if text.contains('\n') => Some(Zeroizing::new(text.clone())),
            _ => None,
        };
        let fields = match v {
            Node::Map { pairs, .. } if top && text == "fields" => build(pairs, end, false)?,
            _ => Vec::new(),
        };
        out.push(KeyLine {
            key: text.clone(),
            line: *line,
            plain: *style == ScalarStyle::Plain,
            end,
            literal,
            fields,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(s: &str) -> Vec<KeyLine> {
        key_layout(s.as_bytes()).expect("a mapping")
    }

    #[test]
    fn reports_each_top_level_key_with_its_line_and_extent() {
        let l = layout("title: X\nvalue: v\nupdated_by: me\n");
        let got: Vec<_> = l.iter().map(|k| (k.key.as_str(), k.line, k.end)).collect();
        assert_eq!(
            got,
            [
                ("title", 0, Some(1)),
                ("value", 1, Some(2)),
                ("updated_by", 2, None)
            ]
        );
        assert!(l.iter().all(|k| k.plain && k.literal.is_none()));
    }

    #[test]
    fn a_literal_block_reports_its_content_and_runs_to_the_next_key() {
        let l = layout("title: X\nvalue: |\n  one\n  two\nfields:\n  a: b\n");
        assert_eq!(
            l[1].literal.as_deref().map(String::as_str),
            Some("one\ntwo\n")
        );
        assert_eq!((l[1].line, l[1].end), (1, Some(4)));
    }

    #[test]
    fn content_that_looks_like_a_key_is_not_one() {
        let l = layout("value: |\n  note: looks like yaml\n  second\nfields:\n  u: v\n");
        let keys: Vec<_> = l.iter().map(|k| k.key.as_str()).collect();
        assert_eq!(keys, ["value", "fields"]);
        assert_eq!(l[1].line, 3);
    }

    #[test]
    fn fields_children_are_reported_and_the_last_inherits_the_parents_end() {
        let l = layout("fields:\n  a: |\n    x\n    y\n  b: c\nvalue: v\n");
        let f = &l[0].fields;
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].key.as_str(), f[0].line, f[0].end), ("a", 1, Some(4)));
        assert_eq!((f[1].key.as_str(), f[1].line, f[1].end), ("b", 4, Some(5)));
        assert!(f[0].literal.is_some());
    }

    #[test]
    fn a_quoted_key_is_reported_as_not_plain() {
        let l = layout("fields:\n  'weird: key': v\n  user: u\n");
        let f = &l[0].fields;
        assert!(!f[0].plain);
        assert_eq!(f[0].key, "weird: key");
        assert!(f[1].plain);
    }

    #[test]
    fn only_the_top_level_fields_key_has_children() {
        let l = layout("value: v\nother:\n  fields:\n    a: b\n");
        assert!(l.iter().all(|k| k.fields.is_empty()));
    }

    #[test]
    fn anything_that_is_not_a_mapping_has_no_layout() {
        for s in [
            "- just\n- a sequence\n",
            "a plain scalar\n",
            "title: [this is not\nvalid: yaml\n",
            "",
            "? [a]\n: b\n",
        ] {
            assert!(key_layout(s.as_bytes()).is_none(), "{s:?}");
        }
        assert!(key_layout(&[0xff, 0xfe]).is_none(), "not UTF-8");
    }
}
