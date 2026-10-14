//! The `$EDITOR` round trip `edit` and `insert -e` share (Go's
//! `editor.go` and `scratchdir_*.go`): write an entry's YAML to a scratch
//! file, run `$EDITOR` on it, read it back. It has no library counterpart
//! on purpose: a GUI edits fields directly rather than shelling out to a
//! text editor ("What's deliberately CLI-only" in the design doc).
//!
//! It is the one place gage puts a decrypted entry on disk, so (Q-EDITOR):
//! `$EDITOR` is resolved before any file exists; the file is created
//! exclusively, with an unpredictable name, 0600 on Unix, in a tmpfs
//! directory where Linux can verify one; and on every way out once it
//! exists, its bytes are overwritten with zeros and it is unlinked.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use gage::entry::{self, Entry};
use gage::exitcode::Code;
use zeroize::Zeroizing;

use crate::error::CliError;

/// What came back from the editor.
pub(crate) struct Edited {
    pub(crate) entry: Entry,
    /// The file came back byte for byte as written. `edit` ignores it (it
    /// re-stamps and commits regardless); `insert -e` aborts on it.
    pub(crate) unchanged: bool,
}

/// Opens `seed`'s YAML in `editor` (`$EDITOR`'s value) and parses what it
/// saves.
pub(crate) fn edit_yaml(editor: Option<&str>, seed: &Entry) -> Result<Edited, CliError> {
    edit_yaml_in(&scratch_dir(), editor, seed)
}

/// [`edit_yaml`] with the scratch file in `dir`.
fn edit_yaml_in(dir: &Path, editor: Option<&str>, seed: &Entry) -> Result<Edited, CliError> {
    let (program, args) = resolve_editor(editor)?;
    let template = entry::marshal(seed)?;
    // From here on the plaintext is on disk; `scratch`'s Drop overwrites
    // and unlinks it on every path out, an error or a panic included.
    let scratch = Scratch::create(dir, &template)?;
    let status = Command::new(&program)
        .args(&args)
        .arg(&scratch.path)
        .status()
        .map_err(|e| CliError::new(Code::Internal, format!("gage: running $EDITOR: {e}")))?;
    if !status.success() {
        return Err(CliError::new(
            Code::Internal,
            format!("gage: $EDITOR exited with an error: {status}"),
        ));
    }
    let edited = Zeroizing::new(fs::read(&scratch.path).map_err(|e| {
        CliError::new(
            Code::Internal,
            format!("gage: reading the edited scratch file: {e}"),
        )
    })?);
    let entry = entry::unmarshal(&edited)?;
    Ok(Edited {
        entry,
        unchanged: *edited == *template,
    })
}

/// `$EDITOR` as a program and its arguments, checked before anything
/// touches the filesystem: unset, blank, or naming nothing `PATH` can find
/// fails here, rather than as a failed spawn after the plaintext is
/// already written. Split on whitespace, which covers `vim` and `code
/// --wait` without a shell-quoting parser.
fn resolve_editor(editor: Option<&str>) -> Result<(PathBuf, Vec<String>), CliError> {
    let editor = editor.unwrap_or("").trim();
    let mut words = editor.split_whitespace();
    let Some(first) = words.next() else {
        return Err(CliError::new(
            Code::Internal,
            "gage: $EDITOR is not set; gage edit and gage insert -e need it to open a scratch file",
        ));
    };
    let program = which::which(first).map_err(|e| {
        CliError::new(
            Code::Internal,
            format!("gage: $EDITOR {editor:?} is not usable: {e}"),
        )
    })?;
    Ok((program, words.map(str::to_string).collect()))
}

/// A scratch file holding plaintext, overwritten and unlinked when
/// dropped.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Creates a fresh file in `dir` (exclusively, with an unpredictable
    /// `gage-edit-*.yaml` name, 0600 on Unix) holding `data`.
    fn create(dir: &Path, data: &[u8]) -> Result<Scratch, CliError> {
        let creating = |e: std::io::Error| {
            CliError::new(Code::Internal, format!("gage: creating scratch file: {e}"))
        };
        let mut builder = tempfile::Builder::new();
        builder.prefix("gage-edit-").suffix(".yaml");
        #[cfg(unix)]
        builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
        let (mut file, path) = builder
            .tempfile_in(dir)
            .map_err(creating)?
            .keep()
            .map_err(|e| creating(e.error))?;
        let scratch = Scratch { path };
        file.write_all(data).map_err(|e| {
            CliError::new(Code::Internal, format!("gage: writing scratch file: {e}"))
        })?;
        // Closed before the editor opens it: Windows won't share it
        // otherwise.
        drop(file);
        Ok(scratch)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = overwrite_then_remove(&self.path);
    }
}

/// Zeroes `path`'s contents in place, keeping its length, so the plaintext
/// is no longer readable through the filesystem. Best effort, not secure
/// erase: a copy-on-write or log-structured filesystem may keep the old
/// blocks anyway, which is why Linux prefers a tmpfs scratch directory.
fn overwrite_file(path: &Path) -> std::io::Result<()> {
    let len = fs::metadata(path)?.len();
    let mut f = OpenOptions::new().write(true).open(path)?;
    let zeros = [0u8; 4096];
    let mut left = len;
    while left > 0 {
        let n = left.min(zeros.len() as u64) as usize;
        f.write_all(&zeros[..n])?;
        left -= n as u64;
    }
    f.sync_all()
}

/// Overwrites `path`, then unlinks it. The overwrite is best effort: the
/// unlink happens whether or not it worked, since by now the caller is on
/// its way out and leaving the file is the worse outcome.
fn overwrite_then_remove(path: &Path) -> std::io::Result<()> {
    let _ = overwrite_file(path);
    fs::remove_file(path)
}

/// TMPFS_MAGIC from linux/magic.h: statfs's `f_type` for a tmpfs mount.
#[cfg(target_os = "linux")]
const TMPFS_MAGIC: i64 = 0x0102_1994;

/// Where scratch files go on Linux: the first of `$XDG_RUNTIME_DIR` and
/// `/dev/shm` that `statfs` verifies as tmpfs, so a plaintext entry is
/// never on a disk-backed filesystem a swap-out or a crash could leave it
/// on. Verified, not assumed from the variable being set. The OS temp
/// directory only when neither verifies.
#[cfg(target_os = "linux")]
pub(crate) fn scratch_dir() -> PathBuf {
    let candidates = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from("/dev/shm")]);
    for dir in candidates {
        if is_tmpfs(&dir) {
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Off Linux: the OS's per-user temp directory. Neither macOS nor Windows
/// gives an unprivileged, portable way to verify a memory-backed
/// filesystem, so this is the narrower guarantee, not a claim of parity
/// with Linux.
#[cfg(not(target_os = "linux"))]
pub(crate) fn scratch_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Whether `dir` is on a tmpfs mount. Anything statfs can't answer for (a
/// path that doesn't exist, say) is not.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn is_tmpfs(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and outlives the call; `st` is
    // valid for one `struct statfs` to be written into.
    if unsafe { libc::statfs(path.as_ptr(), st.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: statfs returned 0, so it filled `st`.
    let st = unsafe { st.assume_init() };
    // f_type's width varies by architecture and libc.
    #[allow(clippy::unnecessary_cast)]
    let fs_type = st.f_type as i64;
    fs_type == TMPFS_MAGIC
}

/// A fake `$EDITOR` for tests: the test binary itself, re-executed to run
/// only [`fake::fake_editor_trampoline`], told what to do by a config file
/// named on its command line (Go's `setFakeEditor`, with a file where Go
/// sets environment variables: in-process tests share one environment).
/// libtest takes the trailing arguments, the config file and the scratch
/// path, as extra name filters that match nothing.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use gage::entry::{self, Entry, Timestamp};

    /// The config file's suffix: how the trampoline knows it was run as an
    /// editor rather than as part of the suite.
    const CONFIG_SUFFIX: &str = ".gage-fake-editor";

    /// One configured fake editor, and where it reports what it saw.
    pub(crate) struct FakeEditor {
        _dir: tempfile::TempDir,
        /// The `$EDITOR` value that runs it.
        pub(crate) command: String,
        record: PathBuf,
        template: PathBuf,
        witness: PathBuf,
    }

    /// What to do to the scratch file.
    pub(crate) enum Action<'a> {
        /// Leave it exactly as written.
        Noop,
        /// Exit 1 without touching it.
        Fail,
        /// Replace it with text that isn't YAML.
        InvalidYaml,
        /// Delete it, so reading it back fails.
        Delete,
        /// Hard-link it to [`FakeEditor::witness`] and leave it, so what
        /// happens to its inode after the editor exits stays observable.
        LinkWitness,
        /// Parse it, apply these, write it back.
        Set(&'a [Set<'a>]),
    }

    /// One change a [`Action::Set`] editor makes.
    pub(crate) enum Set<'a> {
        Title(&'a str),
        Description(&'a str),
        Value(&'a str),
        /// Replaces `fields` with these.
        Fields(&'a [(&'a str, &'a str)]),
        /// Rewrites the stamps gage owns to values it must overrule
        /// (`updated`, `updated_by`) or ignore (`created`).
        TamperStamps,
    }

    impl FakeEditor {
        pub(crate) fn new(action: Action) -> FakeEditor {
            let dir = tempfile::tempdir().unwrap();
            let record = dir.path().join("record.txt");
            let template = dir.path().join("template.yaml");
            let witness = dir.path().join("witness.yaml");
            let mut lines = vec![
                format!("record={}", record.display()),
                format!("template={}", template.display()),
                format!("witness={}", witness.display()),
            ];
            match action {
                Action::Noop => lines.push("mode=noop".into()),
                Action::Fail => lines.push("mode=fail".into()),
                Action::InvalidYaml => lines.push("mode=invalid-yaml".into()),
                Action::Delete => lines.push("mode=delete".into()),
                Action::LinkWitness => lines.push("mode=link-witness".into()),
                Action::Set(sets) => {
                    lines.push("mode=set".into());
                    for s in sets {
                        lines.push(match s {
                            Set::Title(t) => format!("title={t}"),
                            Set::Description(d) => format!("description={d}"),
                            Set::Value(v) => format!("value={v}"),
                            Set::Fields(fields) => format!(
                                "fields={}",
                                fields
                                    .iter()
                                    .map(|(k, v)| format!("{k}={v}"))
                                    .collect::<Vec<_>>()
                                    .join(";")
                            ),
                            Set::TamperStamps => "tamper-stamps=1".into(),
                        });
                    }
                }
            }
            let config = dir.path().join(format!("config{CONFIG_SUFFIX}"));
            std::fs::write(&config, lines.join("\n")).unwrap();
            let command = format!(
                "{} editor::fake::fake_editor_trampoline --exact --nocapture --quiet --test-threads=1 {}",
                std::env::current_exe().unwrap().display(),
                config.display()
            );
            assert_eq!(
                command.split_whitespace().count(),
                7,
                "a path has whitespace in it; $EDITOR is split on whitespace: {command}"
            );
            FakeEditor {
                _dir: dir,
                command,
                record,
                template,
                witness,
            }
        }

        /// The scratch file's path as the editor was given it, or `None`
        /// if the editor never ran.
        pub(crate) fn scratch_path(&self) -> Option<PathBuf> {
            let text = std::fs::read_to_string(&self.record).ok()?;
            Some(PathBuf::from(text.lines().next()?))
        }

        /// The scratch file's Unix permission bits as the editor saw them
        /// (`"600"`), or `"?"` elsewhere.
        pub(crate) fn scratch_mode(&self) -> String {
            let text = std::fs::read_to_string(&self.record).unwrap();
            text.lines().nth(1).unwrap().to_string()
        }

        /// A second name for the scratch file's inode ([`Action::LinkWitness`]),
        /// or `None` if hard links aren't available here.
        pub(crate) fn witness(&self) -> Option<Vec<u8>> {
            std::fs::read(&self.witness).ok()
        }

        /// The file's contents as the editor first saw them.
        pub(crate) fn template(&self) -> Vec<u8> {
            std::fs::read(&self.template).expect("the fake editor never ran")
        }
    }

    /// The editor side. Run as part of the suite, it finds no config
    /// argument and returns.
    #[test]
    fn fake_editor_trampoline() {
        let args: Vec<String> = std::env::args().collect();
        let Some(config) = args.iter().find(|a| a.ends_with(CONFIG_SUFFIX)) else {
            return;
        };
        let scratch = PathBuf::from(args.last().unwrap());
        std::process::exit(run(Path::new(config), &scratch));
    }

    fn run(config: &Path, path: &Path) -> i32 {
        let text = std::fs::read_to_string(config).unwrap();
        let cfg: BTreeMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).collect();
        let mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(path)
                    .map(|m| format!("{:o}", m.permissions().mode() & 0o777))
                    .unwrap_or_else(|_| "?".into())
            }
            #[cfg(not(unix))]
            {
                "?".to_string()
            }
        };
        std::fs::write(cfg["record"], format!("{}\n{mode}\n", path.display())).unwrap();
        let original = std::fs::read(path).unwrap();
        std::fs::write(cfg["template"], &original).unwrap();

        match cfg["mode"] {
            "noop" => 0,
            "fail" => 1,
            "delete" => {
                std::fs::remove_file(path).unwrap();
                0
            }
            // A failed link leaves no witness, which the test reads as
            // "unavailable here".
            "link-witness" => {
                let _ = std::fs::hard_link(path, cfg["witness"]);
                0
            }
            "invalid-yaml" => {
                std::fs::write(path, "title: [this is not\nvalid: yaml").unwrap();
                0
            }
            "set" => {
                let mut e: Entry = entry::unmarshal(&original).unwrap();
                if let Some(t) = cfg.get("title") {
                    e.title = t.to_string();
                }
                if let Some(d) = cfg.get("description") {
                    e.description = d.to_string();
                }
                if let Some(v) = cfg.get("value") {
                    e.value = v.to_string();
                }
                if let Some(fields) = cfg.get("fields") {
                    e.fields = fields
                        .split(';')
                        .filter_map(|kv| kv.split_once('='))
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();
                }
                if cfg.contains_key("tamper-stamps") {
                    let y1999 = Timestamp::from_unix(915_148_800);
                    e.created = y1999;
                    e.updated = y1999;
                    e.updated_by = "some-other-device".into();
                }
                std::fs::write(path, entry::marshal(&e).unwrap()).unwrap();
                0
            }
            other => panic!("unknown fake editor mode {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Action, FakeEditor, Set};
    use super::*;

    fn seed() -> Entry {
        let now = entry::Timestamp::from_unix(1_768_386_720);
        Entry {
            title: "ProtonMail".into(),
            description: "personal email".into(),
            created: now,
            updated: now,
            updated_by: "laptop-1".into(),
            ..Entry::default()
        }
    }

    /// A private scratch directory, so a test can see exactly which files
    /// were left in it without racing the rest of the suite.
    fn in_private_dir(
        editor: Option<&str>,
        seed: &Entry,
    ) -> (tempfile::TempDir, Result<Edited, CliError>) {
        let dir = tempfile::tempdir().unwrap();
        let r = edit_yaml_in(dir.path(), editor, seed);
        (dir, r)
    }

    fn is_empty(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().next().is_none()
    }

    // Go: TestEditYAMLUnsetEditorFailsBeforeAnyScratchFile
    #[test]
    fn edit_yaml_unset_editor_fails_before_any_scratch_file() {
        for editor in [None, Some(""), Some("   \t ")] {
            let (dir, r) = in_private_dir(editor, &seed());
            let err = r.err().expect("an unset $EDITOR must fail");
            assert_eq!(err.code(), Code::Internal);
            assert!(err.to_string().contains("$EDITOR is not set"), "{err}");
            assert!(
                is_empty(dir.path()),
                "{editor:?}: a scratch file was created"
            );
        }
    }

    // Go: TestEditYAMLUnusableEditorFailsBeforeAnyScratchFile
    #[test]
    fn edit_yaml_unusable_editor_fails_before_any_scratch_file() {
        let missing = format!(
            "gage-test-definitely-not-a-real-editor-{} --wait",
            std::process::id()
        );
        let (dir, r) = in_private_dir(Some(&missing), &seed());
        let err = r.err().expect("an unusable $EDITOR must fail");
        assert_eq!(err.code(), Code::Internal);
        assert!(err.to_string().contains("is not usable"), "{err}");
        assert!(is_empty(dir.path()), "a scratch file was created");
    }

    // Go: TestEditYAMLScratchFileCreated0600. As Go, Unix only: Windows has
    // no mode bits to check.
    #[cfg(unix)]
    #[test]
    fn edit_yaml_scratch_file_created_0600() {
        let fake = FakeEditor::new(Action::Noop);
        edit_yaml(Some(&fake.command), &seed()).unwrap();
        assert_eq!(fake.scratch_mode(), "600");
    }

    // Go: TestEditYAMLScratchFileLandsInScratchDir. Through the public
    // entry point, so scratch_dir() can't be right and unused.
    #[test]
    fn edit_yaml_scratch_file_lands_in_scratch_dir() {
        let fake = FakeEditor::new(Action::Noop);
        edit_yaml(Some(&fake.command), &seed()).unwrap();
        let path = fake.scratch_path().expect("the editor ran");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("gage-edit-") && name.ends_with(".yaml"),
            "{name}"
        );
        // Canonical on both sides: macOS's temp dir is behind a symlink.
        assert_eq!(
            fs::canonicalize(path.parent().unwrap()).unwrap(),
            fs::canonicalize(scratch_dir()).unwrap()
        );
    }

    // Go: TestEditYAMLSetValueReportsChanged
    #[test]
    fn edit_yaml_set_value_reports_changed() {
        let fake = FakeEditor::new(Action::Set(&[Set::Value("new-secret")]));
        let (_dir, r) = in_private_dir(Some(&fake.command), &seed());
        let edited = r.unwrap();
        assert!(!edited.unchanged);
        assert_eq!(edited.entry.value, "new-secret");
        assert_eq!(edited.entry.title, "ProtonMail");
    }

    // Go: TestEditYAMLUnchangedReportsUnchangedAndParsesBackIdentically. Also
    // checks what the editor was handed is the seed's YAML.
    #[test]
    fn edit_yaml_unchanged_reports_unchanged_and_parses_back_identically() {
        let fake = FakeEditor::new(Action::Noop);
        let seed = seed();
        let (_dir, r) = in_private_dir(Some(&fake.command), &seed);
        let edited = r.unwrap();
        assert!(edited.unchanged);
        assert_eq!(edited.entry, seed);
        assert_eq!(fake.template(), entry::marshal(&seed).unwrap().to_vec());
    }

    // Go: TestEditYAMLNonZeroExitFails
    #[test]
    fn edit_yaml_non_zero_exit_fails() {
        let fake = FakeEditor::new(Action::Fail);
        let (_dir, r) = in_private_dir(Some(&fake.command), &seed());
        let err = r.err().expect("a failing editor must fail the edit");
        assert_eq!(err.code(), Code::Internal);
        assert!(
            err.to_string().contains("$EDITOR exited with an error"),
            "{err}"
        );
    }

    // Go: TestEditYAMLInvalidYAMLFailsToParse
    #[test]
    fn edit_yaml_invalid_yaml_fails_to_parse() {
        let fake = FakeEditor::new(Action::InvalidYaml);
        let (_dir, r) = in_private_dir(Some(&fake.command), &seed());
        let err = r.err().expect("unparseable content must fail the edit");
        assert_ne!(err.code(), Code::Success);
        assert!(err.to_string().contains("parsing entry"), "{err}");
    }

    // Go: TestEditYAMLScratchFileRemovedOnEveryExitPath. Through the public
    // entry point (the path the editor saw is gone) and in a private
    // directory (nothing at all is left behind in it).
    #[test]
    fn edit_yaml_scratch_file_removed_on_every_exit_path() {
        let actions = || {
            [
                ("noop", Action::Noop),
                ("set-value", Action::Set(&[Set::Value("x")])),
                ("fail", Action::Fail),
                ("invalid-yaml", Action::InvalidYaml),
                ("delete", Action::Delete),
            ]
        };
        for (name, action) in actions() {
            let fake = FakeEditor::new(action);
            let _ = edit_yaml(Some(&fake.command), &seed());
            let path = fake.scratch_path().expect("the editor ran");
            assert!(!path.exists(), "{name}: {} survived", path.display());
        }
        for (name, action) in actions() {
            let fake = FakeEditor::new(action);
            let (dir, _) = in_private_dir(Some(&fake.command), &seed());
            assert!(fake.scratch_path().is_some(), "{name}: the editor ran");
            assert!(is_empty(dir.path()), "{name}: a file was left behind");
        }
    }

    // Not a Go row: the editor removing the file fails the read-back
    // cleanly, and the cleanup that follows has nothing to remove.
    #[test]
    fn edit_yaml_fails_cleanly_when_the_editor_deletes_the_file() {
        let fake = FakeEditor::new(Action::Delete);
        let (dir, r) = in_private_dir(Some(&fake.command), &seed());
        let err = r.err().expect("a vanished file must fail the edit");
        assert_eq!(err.code(), Code::Internal);
        assert!(
            err.to_string().contains("reading the edited scratch file"),
            "{err}"
        );
        assert!(is_empty(dir.path()));
    }

    // Not a Go row: the real flow, not just the helper, overwrites before
    // it unlinks. The fake editor hard-links the scratch file to a witness
    // that outlives the unlink; a cleanup reduced to a bare remove would
    // leave the plaintext readable there.
    #[test]
    fn edit_yaml_overwrites_the_scratch_file_before_unlinking_it() {
        let fake = FakeEditor::new(Action::LinkWitness);
        let mut seed = seed();
        seed.value = "correcthorsebatterystaple".into();
        let (dir, r) = in_private_dir(Some(&fake.command), &seed);
        r.unwrap();
        assert!(is_empty(dir.path()));
        let Some(witness) = fake.witness() else {
            eprintln!("skipped: hard links unavailable here");
            return;
        };
        let template = entry::marshal(&seed).unwrap();
        assert_eq!(witness.len(), template.len(), "overwritten in place");
        assert!(witness.iter().all(|b| *b == 0), "the plaintext survived");
    }

    // Go: TestOverwriteFileZeroesContentsInPlace
    #[test]
    fn overwrite_file_zeroes_contents_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scratch.yaml");
        // Longer than one 4096-byte block of zeros.
        let secret = "value: correcthorsebatterystaple\n".repeat(200);
        fs::write(&path, &secret).unwrap();

        overwrite_file(&path).unwrap();

        let got = fs::read(&path).unwrap();
        assert_eq!(got.len(), secret.len(), "in place, not truncated");
        assert!(got.iter().all(|b| *b == 0), "every byte zeroed");
    }

    // Go: TestOverwriteThenRemoveOverwritesBeforeUnlinking. A hard link
    // keeps the inode reachable after the unlink, so the order is
    // observable: zeroed if the overwrite ran first, plaintext if not.
    #[test]
    fn overwrite_then_remove_overwrites_before_unlinking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scratch.yaml");
        let witness = dir.path().join("witness.yaml");
        let secret = "value: correcthorsebatterystaple\n";
        fs::write(&path, secret).unwrap();
        if let Err(e) = fs::hard_link(&path, &witness) {
            // As Go: the property holds everywhere, this way of observing
            // it doesn't.
            eprintln!("skipped: hard links unavailable here: {e}");
            return;
        }

        overwrite_then_remove(&path).unwrap();

        assert!(!path.exists());
        let got = fs::read(&witness).unwrap();
        assert_eq!(
            got,
            vec![0u8; secret.len()],
            "the plaintext survived the unlink"
        );
    }

    // Not a Go row: a file the editor deleted is not an error worth
    // reporting over, and nothing is left either way.
    #[test]
    fn overwrite_then_remove_of_a_missing_file_only_fails_the_unlink() {
        let dir = tempfile::tempdir().unwrap();
        let err = overwrite_then_remove(&dir.path().join("gone")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    // Go: TestScratchDirIsAWritableDirectory
    #[test]
    fn scratch_dir_is_a_writable_directory() {
        let dir = scratch_dir();
        assert!(!dir.as_os_str().is_empty());
        assert!(fs::metadata(&dir).unwrap().is_dir(), "{}", dir.display());
        tempfile::Builder::new()
            .prefix("gage-scratchdir-test-")
            .tempfile_in(&dir)
            .unwrap();
    }

    // Go: TestScratchDirPrefersTmpfsOnLinux. Skipped where neither
    // candidate is tmpfs (some minimal containers have no /dev/shm).
    #[cfg(target_os = "linux")]
    #[test]
    fn scratch_dir_prefers_tmpfs_on_linux() {
        let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let available = xdg.as_deref().is_some_and(is_tmpfs) || is_tmpfs(Path::new("/dev/shm"));
        if !available {
            eprintln!("skipped: no tmpfs-backed directory on this host");
            return;
        }
        let dir = scratch_dir();
        assert!(is_tmpfs(&dir), "{} is not tmpfs", dir.display());
        assert_ne!(dir, std::env::temp_dir());
    }

    // Go: TestIsTmpfsRejectsANonExistentDirectory
    #[cfg(target_os = "linux")]
    #[test]
    fn is_tmpfs_rejects_a_non_existent_directory() {
        assert!(!is_tmpfs(Path::new(
            "/gage-test-this-path-should-never-exist-anywhere"
        )));
        assert!(!is_tmpfs(Path::new("bad\0path")));
    }

    // Go: TestScratchDirIsTheOSTempDirOffLinux
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn scratch_dir_is_the_os_temp_dir_off_linux() {
        assert_eq!(scratch_dir(), std::env::temp_dir());
    }
}
