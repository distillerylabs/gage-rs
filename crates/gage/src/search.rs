//! `search`/`grep`: a case-insensitive substring match over an entry's
//! title and description, then its value and field values. A result says
//! which entry matched and whether the match was in its body, never what
//! the matched text was.
//!
//! One-shot, [`Vault::search`] decrypts everything fresh. A session
//! matches title and description from its metadata index and decrypts
//! only for the body half, which is never cached (see `Session::search`).
//! The two must classify every entry the same way.

use uuid::Uuid;

use crate::entry::{Entry, Error};
use crate::identity::Identity;
use crate::vault::Vault;

/// One match: enough to render a line without the matched secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    /// The match came from the value or a field rather than the title or
    /// description. Where it matched, never what.
    pub matched_body: bool,
}

/// Whether `lower_pattern` is in the title or description, ignoring case.
pub(crate) fn matches_text(title: &str, description: &str, lower_pattern: &str) -> bool {
    title.to_lowercase().contains(lower_pattern)
        || description.to_lowercase().contains(lower_pattern)
}

/// Whether `lower_pattern` is in the value or any field value, ignoring
/// case.
pub(crate) fn matches_body(e: &Entry, lower_pattern: &str) -> bool {
    e.value.to_lowercase().contains(lower_pattern)
        || e.fields
            .values()
            .any(|v| v.to_lowercase().contains(lower_pattern))
}

/// Title then id: the order `ls` and candidate lists use.
pub(crate) fn sort_results(results: &mut [SearchResult]) {
    results.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
}

impl Vault {
    /// Every entry `pattern` matches, decrypting the whole vault. A title
    /// or description match wins over a body match for the same entry.
    pub fn search(&self, identity: &Identity, pattern: &str) -> Result<Vec<SearchResult>, Error> {
        let lower = pattern.to_lowercase();
        let mut results: Vec<SearchResult> = self
            .decrypt_all(identity)?
            .into_iter()
            .filter_map(|(id, e)| {
                let matched_body = if matches_text(&e.title, &e.description, &lower) {
                    false
                } else if matches_body(&e, &lower) {
                    true
                } else {
                    return None;
                };
                Some(SearchResult {
                    id,
                    title: e.title.clone(),
                    description: e.description.clone(),
                    matched_body,
                })
            })
            .collect();
        sort_results(&mut results);
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::NewEntry;
    use crate::prompter::FakePrompter;
    use crate::session_tests::{Device, unlocks};

    // Go: TestSearchMatchesFieldValueAndReportsMatchedBody. A match only in
    // a field value counts, is flagged as a body match, and carries no
    // matched text.
    #[test]
    fn search_matches_field_value_and_reports_matched_body() {
        let d = Device::new(&["personal"]);
        let v = d.open("personal").unwrap();
        let ident = v.unlock(&unlocks(1)).unwrap();
        for (title, value, field) in [
            (
                "Cloud provider",
                "unrelated",
                ("access_key", "AKIA-needle-1234"),
            ),
            (
                "Nothing to do with it",
                "also unrelated",
                ("note", "no match here"),
            ),
        ] {
            v.insert(
                &ident,
                NewEntry {
                    title: title.into(),
                    value: value.into(),
                    fields: [(field.0.to_string(), field.1.to_string())].into(),
                    ..NewEntry::default()
                },
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        }

        let results = v.search(&ident, "NEEDLE").unwrap();
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(results[0].title, "Cloud provider");
        assert!(results[0].matched_body);
        assert!(!format!("{results:?}").contains("AKIA"), "leaked the match");
    }

    // Go: TestSortSearchResultsTiesByID
    #[test]
    fn sort_search_results_ties_by_id() {
        let lo = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let hi = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let result = |id| SearchResult {
            id,
            title: "same".into(),
            description: String::new(),
            matched_body: false,
        };
        let mut results = vec![result(hi), result(lo)];
        sort_results(&mut results);
        assert_eq!(results.iter().map(|r| r.id).collect::<Vec<_>>(), [lo, hi]);
    }
}
