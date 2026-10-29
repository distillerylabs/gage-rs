//! Queries through the real CLI, in-process: the full query resolution
//! order across every query-taking command, `show`, `edit`, `rename`,
//! `generate` and `insert -e`.
//! `$EDITOR` is a fake (the test binary re-executed, see
//! `editor::fake`) set on the [`Env`].

use crate::exitcode::ExitCode;
use gage::entry::{self, Entry, Timestamp};
use gage::prompter::FakePrompter;
use uuid::Uuid;

use crate::editor::fake::{Action, FakeEditor, Set};
use crate::entry_tests::{DEVICE, ExplodingReader, env_with_vault, unlocks, with_value};
use crate::lifecycle_tests::{CliResult, Env};

impl Env {
    /// Makes `fake` the `$EDITOR` every later command sees. The caller
    /// keeps `fake` alive while they run: its config lives in its temp dir.
    pub(crate) fn set_editor(&self, fake: &FakeEditor) {
        *self.editor.lock().unwrap() = Some(fake.command.clone());
    }

    /// `gage cat <query>`, parsed back into an entry.
    fn cat_entry(&self, query: &str) -> Entry {
        let r = self.cmd_ok(&["cat", query]);
        entry::unmarshal(r.stdout.as_bytes()).unwrap()
    }

    /// Writes an entry under a chosen id through the library, so a test can
    /// pin which substrings its UUID contains (`insert` picks random ones).
    fn write_with_id(&self, id: &str, title: &str, value: &str) -> Uuid {
        let id = Uuid::parse_str(id).unwrap();
        let e = Entry {
            title: title.into(),
            value: value.into(),
            ..Entry::default()
        };
        let vault = self.vault("personal");
        vault.write_entry(id, &e).unwrap();
        // Committed, as a real write would be: an uncommitted file is what
        // the next write's dirty-tree reset discards.
        gage::gitrepo::commit_all(&vault.path, &id.to_string()).unwrap();
        id
    }

    /// Rewrites `title`'s created/updated to a day ago,
    /// so a re-stamp is observably later even when both
    /// commands land in the same wall-clock second.
    fn backdate(&self, title: &str) {
        let vault = self.vault("personal");
        let identity = vault.unlock(&unlocks()).unwrap();
        let (id, mut e) = vault.resolve(&identity, title).unwrap();
        let day_ago = Timestamp::from_unix(e.created.unwrap().as_unix() - 86_400);
        e.created = day_ago;
        e.updated = day_ago;
        vault.update(id, &e, &FakePrompter::new()).unwrap();
    }

    /// Inserts `title` with a value and fields, through `insert -e`.
    fn insert_with_fields(&self, title: &str, value: &str, fields: &[(&str, &str)]) {
        let fake = FakeEditor::new(Action::Set(&[Set::Value(value), Set::Fields(fields)]));
        self.set_editor(&fake);
        self.cmd_ok(&["insert", title, "-e"]);
    }
}

/// Two entries whose titles both contain "aws", neither exactly.
fn two_aws_entries(env: &Env) {
    env.insert_ok("v1", &["AWS root account"]);
    env.insert_ok("v2", &["AWS IAM backup admin"]);
}

fn assert_lists_both_aws_candidates(r: &CliResult) {
    assert_eq!(r.code, ExitCode::Ambiguous, "{}", r.stderr);
    for want in ["AWS root account", "AWS IAM backup admin"] {
        assert!(r.stderr.contains(want), "missing {want}:\n{}", r.stderr);
    }
    assert_eq!(r.stdout, "", "printed to stdout on an ambiguous query");
}

/// The 8-character short id in `edit`'s and `rename`'s confirmations.
fn short(id: Uuid) -> String {
    id.to_string()[..8].to_string()
}

// --- task 3: one resolver behind every query-taking command ----------------

// Rm goes last in each
// case (it deletes), so every form gets a fresh vault.
#[test]
fn show_cat_rm_resolve_identically_across_query_forms() {
    type Form = fn(&str) -> String;
    let forms: [(&str, Form); 7] = [
        ("exact-title", |_| "AWS root account".into()),
        ("unique-substring", |_| "root".into()),
        ("case-insensitive-substring", |_| "ROOT".into()),
        ("exact-uuid", |id| id.into()),
        ("uuid-prefix", |id| id[..8].into()),
        ("short-uuid-prefix", |id| id[..4].into()),
        // Past the first hyphen: a substring, never a prefix.
        ("uuid-mid-string-substring", |id| id[9..17].into()),
    ];
    for (name, form) in forms {
        let env = env_with_vault();
        let id = env.insert_ok("v1", &["AWS root account"]);
        let query = form(&id.to_string());

        let show = env.cmd(&["show", &query]);
        assert_eq!(show.code, ExitCode::Success, "{name}: {}", show.stderr);
        assert_eq!(show.stdout, "v1\n", "{name}");

        assert_eq!(env.cat_entry(&query).title, "AWS root account", "{name}");

        let rm = env.cmd(&["rm", &query]);
        assert_eq!(rm.code, ExitCode::Success, "{name}: {}", rm.stderr);
        assert!(env.entries("personal").is_empty(), "{name}: not removed");
    }
}

// The
// second entry is titled with a real substring of the first's id, so the
// cross-stage collision happens on every run. Compared parsed, not as
// text: an all-digit title is quoted in the YAML.
#[test]
fn cat_resolves_by_exact_title_even_when_query_is_also_a_uuid_substring() {
    let env = env_with_vault();
    let website = env.insert_ok("correcthorsebattery", &["Website Password"]);
    let colliding = website.simple().to_string()[..4].to_string();
    env.insert_ok("some-other-secret", &[&colliding]);

    let got = env.cat_entry(&colliding);

    assert_eq!(got.title, colliding);
    assert_eq!(got.value, "some-other-secret");
}

// The prompter answers the unlock and nothing else, so any prompt would
// panic (there is no candidate picker to call); the editor fails
// if it is ever run, and records that it was.
#[test]
fn ambiguous_query_lists_candidates_and_fails_without_prompting_for_every_command() {
    for args in [
        &["show", "aws"][..],
        &["cat", "aws"],
        &["rm", "aws"],
        &["edit", "aws"],
        &["rename", "aws", "Whatever"],
    ] {
        let env = env_with_vault();
        two_aws_entries(&env);
        let fake = FakeEditor::new(Action::Fail);
        env.set_editor(&fake);
        let prompter = unlocks();

        let r = env.run_with(args, &prompter);

        assert_lists_both_aws_candidates(&r);
        assert!(prompter.value_prompts().is_empty(), "{args:?}");
        assert!(prompter.confirm_prompts().is_empty(), "{args:?}");
        assert!(fake.scratch_path().is_none(), "{args:?} ran the editor");
    }
}

#[test]
fn ambiguous_rm_and_rename_leave_every_candidate_untouched() {
    for args in [&["rm", "aws"][..], &["rename", "aws", "Whatever"]] {
        let env = env_with_vault();
        two_aws_entries(&env);
        let before = env.entries("personal");
        let commits = env.commits("personal");

        assert_eq!(env.cmd(args).code, ExitCode::Ambiguous, "{args:?}");

        let mut after = env.entries("personal");
        let mut before = before.clone();
        before.sort_by_key(|(id, _)| *id);
        after.sort_by_key(|(id, _)| *id);
        assert_eq!(after, before, "{args:?} changed an entry");
        assert_eq!(env.commits("personal"), commits, "{args:?}");
    }
}

// The ids are
// written through the library (insert can't be asked for two that share a
// chosen substring); the behavior under test is the real CLI.
#[test]
fn ambiguous_uuid_substring_fails_at_the_cli_with_candidates() {
    let env = env_with_vault();
    env.write_with_id("abcdef00-1111-4111-8111-111111111111", "Zebra One", "1");
    env.write_with_id("abcdef00-2222-4222-8222-222222222222", "Zebra Two", "2");

    let r = env.cmd(&["cat", "abcdef00"]);

    assert_eq!(r.code, ExitCode::Ambiguous, "{}", r.stderr);
    assert!(r.stderr.contains("Zebra One") && r.stderr.contains("Zebra Two"));
    assert_eq!(r.stdout, "");
}

#[test]
fn unknown_query_is_distinguishable_from_ambiguous_at_the_cli() {
    let env = env_with_vault();
    two_aws_entries(&env);

    let not_found = env.cmd(&["cat", "no such thing at all"]);
    let ambiguous = env.cmd(&["cat", "aws"]);

    assert_eq!(not_found.code, ExitCode::NotFound);
    assert_eq!(ambiguous.code, ExitCode::Ambiguous);
    assert!(
        !not_found.stderr.contains("AWS root account"),
        "a not-found query listed candidates: {}",
        not_found.stderr
    );
}

// --- task 5: show -------------------------------------------------------------

#[test]
fn show_prints_only_value() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["GitHub", "--description", "personal account"]);
    let r = env.cmd_ok(&["show", "GitHub"]);
    assert_eq!(r.stdout, "hunter2\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn show_multiline_value_has_no_extra_trailing_blank_line() {
    let env = env_with_vault();
    let r = env.run_stdin(
        &["insert", "Recovery codes", "-m"],
        &unlocks(),
        b"line1\nline2\n",
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(
        env.cmd_ok(&["show", "Recovery codes"]).stdout,
        "line1\nline2\n"
    );
}

#[test]
fn cat_prints_full_yaml_distinct_from_show() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["GitHub", "--description", "personal account"]);
    let e = env.cat_entry("GitHub");
    assert_eq!(e.title, "GitHub");
    assert_eq!(e.description, "personal account");
    assert_eq!(e.value, "hunter2");
    assert_ne!(env.cmd_ok(&["cat", "GitHub"]).stdout, "hunter2\n");
}

#[test]
fn show_unknown_query_fails_not_found() {
    let env = env_with_vault();
    let r = env.cmd(&["show", "no such thing"]);
    assert_eq!(r.code, ExitCode::NotFound, "{}", r.stderr);
    assert_eq!(r.stdout, "");
}

#[test]
fn show_ambiguous_query_lists_candidates_and_fails() {
    let env = env_with_vault();
    two_aws_entries(&env);
    assert_lists_both_aws_candidates(&env.cmd(&["show", "aws"]));
}

#[test]
fn show_field_extracts_named_field() {
    let env = env_with_vault();
    env.insert_with_fields(
        "Proton",
        "the-password",
        &[
            ("username", "me@example.com"),
            ("totp_seed", "JBSWY3DPEHPK3PXP"),
        ],
    );
    for (name, want) in [
        ("username", "me@example.com"),
        ("totp_seed", "JBSWY3DPEHPK3PXP"),
    ] {
        let r = env.cmd_ok(&["show", "Proton", "--field", name]);
        assert_eq!(r.stdout, format!("{want}\n"), "{name}");
    }
}

#[test]
fn show_unknown_field_fails_clearly() {
    let env = env_with_vault();
    env.insert_with_fields("Proton", "the-password", &[("username", "me@example.com")]);
    let r = env.cmd(&["show", "Proton", "--field", "passwrod"]);
    assert_eq!(r.code, ExitCode::NotFound, "{}", r.stderr);
    assert!(r.stderr.contains("passwrod") && r.stderr.contains("username"));
    assert!(!r.stderr.contains("the-password") && !r.stderr.contains("me@example.com"));
    assert_eq!(r.stdout, "");
}

#[test]
fn show_field_on_entry_with_no_fields() {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["GitHub"]);
    let r = env.cmd(&["show", "GitHub", "--field", "username"]);
    assert_eq!(r.code, ExitCode::NotFound, "{}", r.stderr);
    assert!(r.stderr.contains("no fields"), "{}", r.stderr);
    assert_eq!(r.stdout, "");
}

// --- task 5: edit -------------------------------------------------------------

#[test]
fn edit_changes_value_and_leaves_other_fields_untouched() {
    let env = env_with_vault();
    let id = env.insert_ok(
        "old-value",
        &["ProtonMail", "--description", "personal email"],
    );
    let fake = FakeEditor::new(Action::Set(&[Set::Value("new-value")]));
    env.set_editor(&fake);

    let r = env.cmd_ok(&["edit", "ProtonMail"]);
    fake.assert_scratch_gone();

    assert_eq!(
        r.stdout,
        format!("gage: updated \"ProtonMail\" ({})\n", short(id))
    );
    let got = env.cat_entry("ProtonMail");
    assert_eq!(got.value, "new-value");
    assert_eq!(got.description, "personal email");
    assert_eq!(got.title, "ProtonMail");
}

// Edit has no
// "unchanged" abort: an untouched file still re-stamps and commits.
#[test]
fn edit_restamps_updated_and_updated_by_and_commits() {
    let env = env_with_vault();
    env.insert_ok("v1", &["ProtonMail", "--description", "personal email"]);
    env.backdate("ProtonMail");
    let before = env.cat_entry("ProtonMail");
    let commits = env.commits("personal");
    let _editor = FakeEditor::new(Action::Noop);
    env.set_editor(&_editor);

    env.cmd_ok(&["edit", "ProtonMail"]);

    let after = env.cat_entry("ProtonMail");
    assert!(after.updated > before.updated, "updated bumped");
    assert_eq!(after.updated_by, DEVICE);
    assert_eq!(
        Entry {
            updated: before.updated,
            ..after
        },
        before,
        "nothing else changed"
    );
    assert_eq!(env.commits("personal"), commits + 1);
}

// The editor rewrites
// the stamps gage owns; gage's own win.
#[test]
fn edit_restamp_is_authoritative_over_the_edited_file() {
    let env = env_with_vault();
    env.insert_ok("v1", &["ProtonMail"]);
    env.backdate("ProtonMail");
    let before = env.cat_entry("ProtonMail");
    let _editor = FakeEditor::new(Action::Set(&[Set::TamperStamps]));
    env.set_editor(&_editor);

    env.cmd_ok(&["edit", "ProtonMail"]);

    let after = env.cat_entry("ProtonMail");
    assert_eq!(after.updated_by, DEVICE, "from the identity, not the file");
    assert!(
        after.updated > before.updated,
        "from the clock, not the file"
    );
    assert_eq!(
        after.created, before.created,
        "from the original, not the file"
    );
}

#[test]
fn edit_unparseable_yaml_aborts_with_no_commit_and_original_unchanged() {
    let env = env_with_vault();
    env.insert_ok("original-value", &["ProtonMail"]);
    let before = env.entries("personal");
    let commits = env.commits("personal");
    let fake = FakeEditor::new(Action::InvalidYaml);
    env.set_editor(&fake);

    let r = env.cmd(&["edit", "ProtonMail"]);
    fake.assert_scratch_gone();

    assert_ne!(r.code, ExitCode::Success);
    assert!(r.stderr.starts_with("gage: "), "{}", r.stderr);
    assert_eq!(r.stdout, "");
    assert_eq!(env.commits("personal"), commits);
    assert_eq!(env.entries("personal"), before);
}

#[test]
fn edit_ambiguous_query_lists_candidates_and_fails() {
    let env = env_with_vault();
    two_aws_entries(&env);
    let fake = FakeEditor::new(Action::Noop);
    env.set_editor(&fake);
    assert_lists_both_aws_candidates(&env.cmd(&["edit", "aws"]));
    assert!(fake.scratch_path().is_none(), "the editor was opened");
}

// With no usable $EDITOR, edit fails before writing anything.
#[test]
fn edit_without_an_editor_fails_and_changes_nothing() {
    let env = env_with_vault();
    env.insert_ok("v", &["ProtonMail"]);
    let commits = env.commits("personal");
    let r = env.cmd(&["edit", "ProtonMail"]);
    assert_eq!(r.code, ExitCode::Internal, "{}", r.stderr);
    assert!(r.stderr.contains("$EDITOR is not set"), "{}", r.stderr);
    assert_eq!(env.commits("personal"), commits);
}

// --- task 5: rename -----------------------------------------------------------

#[test]
fn rename_changes_only_title_through_the_cli() {
    let env = env_with_vault();
    let id = env.insert_ok(
        "hunter2",
        &["ProtonMail", "--description", "personal email"],
    );

    let r = env.cmd_ok(&["rename", "ProtonMail", "Proton Mail"]);

    assert_eq!(
        r.stdout,
        format!("gage: renamed to \"Proton Mail\" ({})\n", short(id))
    );
    let got = env.cat_entry("Proton Mail");
    assert_eq!(got.title, "Proton Mail");
    assert_eq!(got.value, "hunter2");
    assert_eq!(got.description, "personal email");
    assert_eq!(env.cmd(&["cat", "ProtonMail"]).code, ExitCode::NotFound);
}

#[test]
fn rename_to_existing_title_rejected_unless_forced_through_the_cli() {
    let env = env_with_vault();
    env.insert_ok("a", &["Entry A"]);
    env.insert_ok("b", &["Entry B"]);

    let rejected = env.cmd(&["rename", "Entry B", "Entry A"]);
    assert_eq!(rejected.code, ExitCode::Conflict, "{}", rejected.stderr);

    env.cmd_ok(&["rename", "Entry B", "Entry A", "--force"]);
    let ls = env.cmd_ok(&["ls"]);
    assert_eq!(ls.stdout.matches("Entry A").count(), 2, "{}", ls.stdout);
}

#[test]
fn rename_ambiguous_query_lists_candidates_and_fails() {
    let env = env_with_vault();
    two_aws_entries(&env);
    assert_lists_both_aws_candidates(&env.cmd(&["rename", "aws", "Whatever"]));
}

// --- task 6: generate ---------------------------------------------------------

// The
// confirmation names the entry and never the value.
#[test]
fn generate_inserts_entry_with_random_value_of_sane_default_length() {
    let env = env_with_vault();

    let r = env.cmd_ok(&["generate", "GitHub"]);

    let (id, e) = env.only_entry("personal");
    assert_eq!(e.title, "GitHub");
    assert_eq!(e.value.len(), gage::generate::DEFAULT_LENGTH);
    assert_eq!(r.stdout, format!("gage: generated \"GitHub\" ({id})\n"));
    assert!(!r.stdout.contains(&e.value) && !r.stderr.contains(&e.value));
}

#[test]
fn generate_produces_different_values_across_entries() {
    let env = env_with_vault();
    env.cmd_ok(&["generate", "Zeta"]);
    env.cmd_ok(&["generate", "Yolk"]);
    assert_ne!(env.cat_entry("Zeta").value, env.cat_entry("Yolk").value);
}

#[test]
fn generate_produces_exactly_one_commit_and_sets_timestamps() {
    let env = env_with_vault();
    let commits = env.commits("personal");
    env.cmd_ok(&["generate", "Site", "--description", "the site"]);
    let e = env.cat_entry("Site");
    assert!(e.created.is_some() && e.created == e.updated);
    assert_eq!(e.updated_by, DEVICE);
    assert_eq!(e.description, "the site");
    assert_eq!(env.commits("personal"), commits + 1);
}

#[test]
fn generate_duplicate_title_rejected_unless_forced() {
    let env = env_with_vault();
    env.cmd_ok(&["generate", "Site"]);
    assert_eq!(env.cmd(&["generate", "Site"]).code, ExitCode::Conflict);
    env.cmd_ok(&["generate", "Site", "--force"]);
    assert_eq!(env.entries("personal").len(), 2);
}

#[test]
fn generate_respects_length_and_no_symbols() {
    let env = env_with_vault();
    env.cmd_ok(&["generate", "Long", "-l", "40"]);
    assert_eq!(env.cat_entry("Long").value.len(), 40);
    env.cmd_ok(&["generate", "Plain", "--no-symbols", "--length", "32"]);
    let v = env.cat_entry("Plain").value;
    assert_eq!(v.len(), 32);
    assert!(v.chars().all(|c| c.is_ascii_alphanumeric()), "{v}");
}

// Before any unlock (the
// prompter has nothing scripted). A negative length is a parse error, also
// Usage.
#[test]
fn generate_rejects_an_unreasonably_small_length() {
    let env = env_with_vault();
    let commits = env.commits("personal");
    for length in ["4", "-5", "lots"] {
        let prompter = FakePrompter::new();
        let r = env.run_with(&["generate", "Weak", "-l", length], &prompter);
        assert_eq!(r.code, ExitCode::Usage, "{length}: {}", r.stderr);
        assert!(prompter.unlock_requests().is_empty(), "{length}");
    }
    let r = env.run_with(&["generate", "Weak", "-l", "4"], &FakePrompter::new());
    assert!(r.stderr.contains("minimum is 8"), "{}", r.stderr);
    assert!(env.entries("personal").is_empty());
    assert_eq!(env.commits("personal"), commits);
}

// --- task 6: insert -e --------------------------------------------------------

#[test]
fn insert_edit_opens_template_and_inserts_edited_value_and_fields() {
    let env = env_with_vault();
    let _editor = FakeEditor::new(Action::Set(&[Set::Value("edited-value")]));
    env.set_editor(&_editor);

    let r = env.cmd_ok(&[
        "insert",
        "ProtonMail",
        "--description",
        "personal email",
        "-e",
    ]);

    let (id, got) = env.only_entry("personal");
    assert_eq!(r.stdout, format!("gage: inserted \"ProtonMail\" ({id})\n"));
    assert_eq!(got.value, "edited-value");
    assert_eq!(got.description, "personal email");
    assert_eq!(got.updated_by, DEVICE);
    assert!(got.created.is_some() && got.created == got.updated);
}

// What the
// editor was handed, recorded by the fake before it changed anything.
#[test]
fn insert_edit_opens_a_pre_filled_template_with_empty_value_and_fields() {
    let env = env_with_vault();
    let fake = FakeEditor::new(Action::Set(&[Set::Value("v")]));
    env.set_editor(&fake);

    env.cmd_ok(&[
        "insert",
        "ProtonMail",
        "--description",
        "personal email",
        "-e",
    ]);

    let template = fake.template();
    let seeded = entry::unmarshal(&template).unwrap();
    assert_eq!(seeded.title, "ProtonMail");
    assert_eq!(seeded.description, "personal email");
    assert_eq!(seeded.value, "");
    assert!(seeded.fields.is_empty());
    let text = String::from_utf8(template).unwrap();
    assert!(!text.contains("fields:"), "an empty fields: key:\n{text}");
}

#[test]
fn insert_edit_sets_fields_at_creation() {
    let env = env_with_vault();
    let _editor = FakeEditor::new(Action::Set(&[
        Set::Value("edited-value"),
        Set::Fields(&[("username", "me@example.com")]),
    ]));
    env.set_editor(&_editor);
    env.cmd_ok(&["insert", "GitHub", "-e"]);
    let got = env.cat_entry("GitHub");
    assert_eq!(got.value, "edited-value");
    assert_eq!(got.field("username").unwrap(), "me@example.com");
}

#[test]
fn insert_edit_aborts_with_no_entry_written_and_no_commit_if_unchanged() {
    let env = env_with_vault();
    let commits = env.commits("personal");
    let fake = FakeEditor::new(Action::Noop);
    env.set_editor(&fake);

    let r = env.cmd(&["insert", "ProtonMail", "-e"]);

    fake.assert_scratch_gone();
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("nothing to save"), "{}", r.stderr);
    assert_eq!(env.commits("personal"), commits);
    assert!(env.entries("personal").is_empty());
}

// A changed
// description alone is not something to save.
#[test]
fn insert_edit_aborts_if_value_and_fields_both_still_empty() {
    let env = env_with_vault();
    let commits = env.commits("personal");
    let fake = FakeEditor::new(Action::Set(&[Set::Description("changed")]));
    env.set_editor(&fake);

    let r = env.cmd(&["insert", "ProtonMail", "-e"]);
    fake.assert_scratch_gone();

    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert_eq!(env.commits("personal"), commits);
    assert!(env.entries("personal").is_empty());
}

#[test]
fn insert_edit_changed_title_used_for_saved_entry_and_duplicate_check() {
    let env = env_with_vault();
    env.insert_ok("existing", &["EditedTitle"]);
    let fake = FakeEditor::new(Action::Set(&[Set::Title("EditedTitle"), Set::Value("v")]));
    env.set_editor(&fake);

    let dup = env.cmd(&["insert", "OriginalTitle", "-e"]);
    fake.assert_scratch_gone();
    assert_eq!(dup.code, ExitCode::Conflict, "{}", dup.stderr);
    assert_eq!(env.cmd(&["cat", "OriginalTitle"]).code, ExitCode::NotFound);

    env.cmd_ok(&["insert", "OriginalTitle", "-e", "--force"]);
    let ls = env.cmd_ok(&["ls"]);
    assert_eq!(ls.stdout.matches("EditedTitle").count(), 2, "{}", ls.stdout);
    assert!(!ls.stdout.contains("OriginalTitle"), "{}", ls.stdout);
}

// Rejected at parse time: nothing is prompted for, read, unlocked or
// opened. The refusal is clap's own ("cannot be used with").
#[test]
fn insert_edit_rejected_alongside_multiline_value_stdin_or_field_before_any_io() {
    let env = env_with_vault();
    let fake = FakeEditor::new(Action::Set(&[Set::Value("v")]));
    env.set_editor(&fake);
    for args in [
        &["insert", "Site", "-e", "-m"][..],
        &["insert", "Site", "-e", "--value-stdin"],
        &["insert", "Site", "--edit", "--field", "user=bob"],
    ] {
        let prompter = FakePrompter::new();
        let r = env.run_reader(args, &prompter, &mut ExplodingReader);
        assert_eq!(r.code, ExitCode::Usage, "{args:?}: {}", r.stderr);
        assert!(r.stderr.contains("cannot be used with"), "{}", r.stderr);
        assert!(prompter.unlock_requests().is_empty(), "{args:?}");
    }
    assert!(fake.scratch_path().is_none(), "the editor was opened");
    assert!(env.entries("personal").is_empty());
}

// --- field flags and a vault without an id -----------------------------------

// The `show --field` halves of the insert-field
// round-trip tests in entry_tests.
#[test]
fn insert_fields_are_each_readable_through_show_field() {
    let env = env_with_vault();
    env.insert_ok(
        "v",
        &[
            "Site",
            "--field",
            "user=bob",
            "--field",
            "totp_seed=JBSWY3DPEHPK3PXP",
        ],
    );
    for (name, want) in [("user", "bob"), ("totp_seed", "JBSWY3DPEHPK3PXP")] {
        let r = env.cmd_ok(&["show", "Site", "--field", name]);
        assert_eq!(r.stdout, format!("{want}\n"), "{name}");
    }
}

// The `show` half of "a vault without an id fails loudly on every derived path"
// (the insert half is in entry_tests): a registration with an empty id
// fails `show` before anything is prompted for or any path is built.
#[test]
fn a_registration_without_an_id_fails_show_before_any_path_is_built() {
    let env = env_with_vault();
    env.insert_ok("v", &["x"]);
    let mut global = env.global();
    global.vaults.get_mut("personal").unwrap().id = None;
    env.save_global(&global);
    let before = env.tree();
    let prompter = FakePrompter::new();

    let r = env.run_with(&["show", "x"], &prompter);

    assert_eq!(r.code, ExitCode::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("points at a different vault"),
        "{}",
        r.stderr
    );
    assert!(prompter.unlock_requests().is_empty());
    assert_eq!(r.stdout, "");
    assert_eq!(env.tree(), before, "no lock or identity path was created");
}

// --- registration and error prefixes ------------------------------------------

// Every failure the new
// commands add, prefixed exactly once.
#[test]
fn no_m5_command_doubles_gages_own_prefix() {
    use crate::error::ERROR_PREFIX;
    let doubled = format!("{ERROR_PREFIX}{ERROR_PREFIX}");
    let env = env_with_vault();
    env.insert_ok("v", &["dup"]);
    two_aws_entries(&env);
    let failing = FakeEditor::new(Action::Fail);
    let cases: &[(&str, &[&str], bool)] = &[
        ("show unknown", &["show", "nope"], false),
        ("show ambiguous", &["show", "aws"], false),
        (
            "show unknown field",
            &["show", "dup", "--field", "x"],
            false,
        ),
        ("edit unknown", &["edit", "nope"], false),
        ("edit ambiguous", &["edit", "aws"], false),
        ("edit no editor", &["edit", "dup"], false),
        ("edit failing editor", &["edit", "dup"], true),
        ("rename unknown", &["rename", "nope", "x"], false),
        ("rename duplicate", &["rename", "aws root", "dup"], false),
        ("rename ambiguous", &["rename", "aws", "x"], false),
        ("generate duplicate", &["generate", "dup"], false),
        ("generate too short", &["generate", "x", "-l", "3"], false),
        ("insert -e no editor", &["insert", "x", "-e"], false),
        ("insert -e failing editor", &["insert", "x", "-e"], true),
        ("insert -e exclusive", &["insert", "x", "-e", "-m"], false),
    ];
    for (name, args, with_editor) in cases {
        *env.editor.lock().unwrap() = with_editor.then(|| failing.command.clone());
        let r = env.run_with(args, &with_value("v"));
        assert_ne!(
            r.code,
            ExitCode::Success,
            "{name}: this table only covers failures"
        );
        for line in r.stdout.lines().chain(r.stderr.lines()) {
            assert!(!line.contains(&doubled), "{name}: {line}");
            assert!(line.matches(ERROR_PREFIX).count() <= 1, "{name}: {line}");
        }
        assert!(
            r.stderr.starts_with(ERROR_PREFIX),
            "{name}: unprefixed:\n{}",
            r.stderr
        );
    }
}
