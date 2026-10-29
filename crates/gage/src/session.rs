//! One or more vaults' unlocked [`Identity`] values held for longer than
//! one command: the library type behind `gage-cli`'s REPL, and any later
//! GUI's "unlock once, do several things" mode. See "Session model" in the
//! design doc.
//!
//! A session adds no memory-protection mechanics of its own. It unlocks
//! with the same [`Vault::unlock`] a one-shot command calls, and locking
//! (explicitly, on idle, or on close) drops the same `Identity`, whose
//! `Drop` wipes and unlocks its pages. Commands call the same `Vault`
//! methods in both modes, handed a `&Identity` borrowed from here: what
//! differs is only how long the identity lives.
//!
//! A re-lock can't pull a key out from under a command still using it: a
//! command's `&Identity` borrows the session, and every way to lock
//! needs `&mut Session`, so that overlap doesn't compile.
//! What matters is *when* the idle timeout runs:
//! on entering a call, never between the calls of one command.
//!
//! Each vault a session has unlocked can also carry a metadata index
//! (title, description, dates; never a value or a field), built by the
//! first `list`, `resolve` or `search` against it and dropped with its
//! key. See `index` and "Addressing entries & the metadata index".
//!
//! A pull or a merge that moves a vault's working tree (on unlock, on
//! `use`, inside a write's push, or through a sync verb) raises the flag
//! the session installed on that vault, and the index is dropped before
//! anything is served from it again.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::entry::{self, Entry, ListEntry, ResolveError, resolve_query};
use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;
use crate::identityfile::UnlockError;
use crate::index::Index;
use crate::prompter::{PromptError, Prompter};
use crate::remotesync::{SyncError, SyncReport};
use crate::search::{SearchResult, matches_body, sort_results};
use crate::vault::Vault;

/// Why the session's opener couldn't produce a vault: whatever the
/// frontend's lookup failed with (an unregistered name, an unreadable
/// config, a registration pointing at another vault), keeping its message
/// and exit code.
#[derive(Debug)]
pub struct OpenError {
    code: Code,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl OpenError {
    pub fn new<E>(err: E) -> OpenError
    where
        E: std::error::Error + HasCode + Send + Sync + 'static,
    {
        OpenError {
            code: err.code(),
            source: Box::new(err),
        }
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

// Transparent: the wrapped error's own source, so nobody renders the same
// message twice.
impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.source()
    }
}

impl HasCode for OpenError {
    fn code(&self) -> Code {
        self.code
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// No vault was named and none is current, or `use` was given an
    /// empty name. Reported before any prompt: there's no vault whose
    /// passphrase could be asked for.
    #[error("gage: no vault is selected in this session: {hint}")]
    NoCurrentVault { hint: &'static str },
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error(transparent)]
    Unlock(#[from] UnlockError),
    /// `lock` named a vault this session never touched.
    #[error("gage: {0:?} is not unlocked in this session")]
    NotHeld(String),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// Reading the entry a human picked failed.
    #[error(transparent)]
    Entry(#[from] entry::Error),
    /// The candidate picker failed or gave up.
    #[error(transparent)]
    Choose(PromptError),
    /// The prompter answered a candidate list with an id that wasn't in
    /// it: a frontend bug, refused rather than trusted, since it would
    /// otherwise select an arbitrary entry.
    #[error("gage: the chosen id {0} is not one of the candidates offered")]
    NotACandidate(Uuid),
}

impl HasCode for SessionError {
    fn code(&self) -> Code {
        match self {
            SessionError::NoCurrentVault { .. } => Code::Usage,
            SessionError::Open(e) => e.code(),
            SessionError::Unlock(e) => e.code(),
            SessionError::NotHeld(_) => Code::NotFound,
            SessionError::Resolve(e) => e.code(),
            SessionError::Entry(e) => e.code(),
            SessionError::Choose(e) => e.code(),
            SessionError::NotACandidate(_) => Code::Internal,
        }
    }
}

/// Turns a vault name into the vault it names. Injected, because which
/// names are registered on this machine is global-config knowledge the
/// frontend already resolves for one-shot mode; a session's job is holding
/// identities, not re-deriving the registry. Only ever called with a
/// non-empty name.
pub type Opener<'a> = Box<dyn Fn(&str) -> Result<Vault, OpenError> + 'a>;

/// Everything [`Session::new`] needs.
pub struct SessionConfig<'a> {
    pub open: Opener<'a>,
    /// What unlocking, and an ambiguous query's candidate list, ask a
    /// human through: the same prompter one-shot mode uses.
    pub prompter: &'a dyn Prompter,
    /// Re-lock a vault untouched for this long. `None` or zero disables
    /// it. See "Why an idle timeout still matters despite process-scoped
    /// keys".
    pub idle_timeout: Option<Duration>,
    /// The vault commands use when none is named: normally global config's
    /// `current`, so a session behaves like one-shot mode until `use`.
    /// Naming it doesn't unlock it; the first call that needs a key does.
    pub current: Option<String>,
    /// The clock the idle timeout reads; `None` is [`Instant::now`].
    /// Injected so tests simulate elapsed time instead of sleeping. A
    /// plain function suffices: the timeout is a check at the start of a
    /// call, never a background timer.
    pub now: Option<Box<dyn Fn() -> Instant + 'a>>,
}

/// Every entry of a vault, decrypted in one pass.
type Decrypted = Vec<(Uuid, Entry)>;

/// One row of [`Session::status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultState {
    pub name: String,
    pub unlocked: bool,
    pub current: bool,
    /// How long since this vault last served a call; `None` if it never
    /// has (its unlock failed).
    pub idle: Option<Duration>,
}

/// One vault this session has touched. It outlives a lock: a locked vault
/// is still one the session knows, which is what lets `status` list it
/// and the prompt show it locked.
struct HeldVault {
    vault: Vault,
    /// `None` is locked.
    identity: Option<Identity>,
    last_used: Option<Instant>,
    /// The metadata index: `None` until a list, resolve or search needs
    /// one, and again whenever the identity is dropped.
    index: Option<Index>,
    /// Whether this index's page-lock failure has been reported, so a
    /// growing arena doesn't repeat it. Reset with the index: a rebuild
    /// may meet a different limit.
    index_warned: bool,
    /// Raised by the vault when a pull or merge moves its working tree:
    /// the index no longer describes `entries/`.
    pulled: Arc<AtomicBool>,
}

pub struct Session<'a> {
    open: Opener<'a>,
    prompter: &'a dyn Prompter,
    idle_timeout: Option<Duration>,
    now: Box<dyn Fn() -> Instant + 'a>,
    current: Option<String>,
    vaults: BTreeMap<String, HeldVault>,
    /// How many commands are in flight (see [`Session::begin_command`]);
    /// the idle timeout only runs at zero.
    depth: usize,
}

impl<'a> Session<'a> {
    pub fn new(cfg: SessionConfig<'a>) -> Session<'a> {
        Session {
            open: cfg.open,
            prompter: cfg.prompter,
            idle_timeout: cfg.idle_timeout.filter(|t| !t.is_zero()),
            now: cfg.now.unwrap_or_else(|| Box::new(Instant::now)),
            current: cfg.current,
            vaults: BTreeMap::new(),
            depth: 0,
        }
    }

    /// The vault commands operate on when none is named.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// The prompter this session asks through, for a command that has a
    /// question of its own (an insert's value).
    pub fn prompter(&self) -> &'a dyn Prompter {
        self.prompter
    }

    /// Makes `name` the current vault, unlocking it unless this session
    /// already holds its key. Re-selecting an unlocked vault never
    /// re-prompts: that is the point of a session.
    ///
    /// `use` means "catch me up" either way. A first `use` unlocks, and the
    /// unlock pulls; re-selecting a vault already unlocked runs no unlock,
    /// so the pull is asked for here rather than skipped for the vaults a
    /// session returns to most.
    pub fn use_vault(&mut self, name: &str) -> Result<(), SessionError> {
        if name.is_empty() {
            return Err(SessionError::NoCurrentVault {
                hint: "use <vault>",
            });
        }
        let was_unlocked = self.vaults.get(name).is_some_and(|h| h.identity.is_some());
        self.vault(Some(name))?;
        if was_unlocked {
            let prompter = self.prompter;
            self.vaults[name].vault.sync_on_unlock(prompter);
        }
        self.current = Some(name.to_string());
        Ok(())
    }

    /// A vault this session can name, without unlocking it: for the sync
    /// verbs, which decrypt nothing for a fast-forward or a merge of
    /// disjoint changes, so asking for a passphrase up front would prompt
    /// for a key most syncs never use. `None` is the current vault. A pull
    /// or merge through it still invalidates this session's index.
    pub fn vault_without_unlocking(&mut self, name: Option<&str>) -> Result<&Vault, SessionError> {
        self.enter();
        let name = self.target(name)?;
        Ok(&self.hold(&name)?.vault)
    }

    /// A source vault with its unlocked identity, and a second vault to
    /// write into, for `mv`/`cp`. The destination is held (registered, so
    /// a later pull still invalidates its index) but never unlocked:
    /// encrypting to its recipients needs no private key. It is opened
    /// first, so a destination that isn't registered fails before the
    /// source asks for a passphrase. One method rather than two
    /// [`Session::vault`] calls because each borrows the whole session.
    pub fn vault_and_destination(
        &mut self,
        source: Option<&str>,
        dest: &str,
    ) -> Result<(&Vault, &Identity, &Vault), SessionError> {
        if dest.is_empty() {
            return Err(SessionError::NoCurrentVault {
                hint: "name the destination vault",
            });
        }
        self.enter();
        self.hold(dest)?;
        self.vault(source)?;
        let source = self.target(source)?;
        let (src, dst) = (&self.vaults[&source], &self.vaults[dest]);
        let identity = src.identity.as_ref().expect("unlocked above");
        Ok((&src.vault, identity, &dst.vault))
    }

    /// `gage sync` with interactive resolution in a vault (`None` is the
    /// current one), through this session's prompter. A conflict reuses
    /// the identity the session already holds, so nobody is asked twice; a
    /// vault not yet unlocked is unlocked once, through the session's own
    /// path, and only if a conflict needs it, and then stays unlocked like
    /// any other. A fast-forward or a disjoint merge unlocks nothing.
    ///
    /// The outer error is the vault lookup; the inner one is the sync's,
    /// with its report.
    pub fn sync_resolving(
        &mut self,
        name: Option<&str>,
    ) -> Result<Result<SyncReport, SyncError>, SessionError> {
        self.enter();
        let name = self.target(name)?;
        let prompter = self.prompter;
        let now = (self.now)();
        let held = self.hold(&name)?;
        // The vault and its cached identity borrowed together: both are
        // shared borrows of one held vault.
        let (vault, cached) = (&held.vault, held.identity.as_ref());
        let unlocked = OnceCell::new();
        let result = vault.sync_resolving_with(prompter, &mut || {
            if let Some(identity) = cached.or(unlocked.get()) {
                return Ok(identity);
            }
            let identity = vault.unlock(prompter)?;
            Ok(unlocked.get_or_init(|| identity))
        });
        if let Some(identity) = unlocked.into_inner() {
            held.identity = Some(identity);
        }
        if held.identity.is_some() {
            held.last_used = Some(now);
        }
        Ok(result)
    }

    /// A vault and its unlocked identity, unlocking first if needed.
    /// `None` is the current vault; naming one (the `-u/--use` path) uses
    /// it without changing which vault is current. The identity stays
    /// the session's: [`Session::lock`], the idle timeout and
    /// [`Session::close`] are what drop it.
    pub fn vault(&mut self, name: Option<&str>) -> Result<(&Vault, &Identity), SessionError> {
        self.enter();
        let name = self.target(name)?;
        let now = (self.now)();
        let prompter = self.prompter;
        let held = self.hold(&name)?;
        if held.identity.is_none() {
            held.identity = Some(held.vault.unlock(prompter)?);
        }
        held.last_used = Some(now);
        let identity = held.identity.as_ref().expect("unlocked above");
        Ok((&held.vault, identity))
    }

    /// Resolves `query` in a vault (`None` is the current one) through the
    /// library's one resolver, and on an ambiguous query asks the prompter
    /// which candidate was meant rather than failing: the whole difference
    /// from one-shot mode, which has nobody to ask. A prompter that
    /// declines (`Cancelled`) gets the candidate list back, as one-shot
    /// does. See "Addressing entries & the metadata index".
    pub fn resolve(
        &mut self,
        name: Option<&str>,
        query: &str,
    ) -> Result<(Uuid, Entry), SessionError> {
        let prompter = self.prompter;
        let (name, _) = self.ensure_index(name)?;
        let held = &self.vaults[&name];
        let index = held.index.as_ref().expect("ensure_index built it");
        let identity = held.identity.as_ref().expect("ensure_index unlocked it");
        let list = match resolve_query(query, &index.titles()) {
            Ok(id) => return Ok((id, held.vault.read_entry(id, identity)?)),
            Err(ResolveError::Ambiguous(list)) => list,
            Err(e) => return Err(e.into()),
        };
        let chosen = match prompter.choose(&list) {
            Ok(id) => id,
            Err(PromptError::Cancelled) => return Err(ResolveError::Ambiguous(list).into()),
            Err(e) => return Err(SessionError::Choose(e)),
        };
        if !list.candidates.iter().any(|c| c.id == chosen) {
            return Err(SessionError::NotACandidate(chosen));
        }
        Ok((chosen, held.vault.read_entry(chosen, identity)?))
    }

    /// A vault's `ls` rows (`None` is the current one), from its index:
    /// the same rows, in the same order, as `Vault::list`.
    pub fn list(&mut self, name: Option<&str>) -> Result<Vec<ListEntry>, SessionError> {
        let (name, _) = self.ensure_index(name)?;
        Ok(self.vaults[&name]
            .index
            .as_ref()
            .expect("ensure_index built it")
            .list())
    }

    /// `search` in a vault: title and description from the index, body
    /// text decrypted fresh and never cached, so a matched secret lives in
    /// memory no longer than a `show` keeps it. A first search builds the
    /// index from the same pass its body check uses. A title or
    /// description match wins over a body match, as in `Vault::search`.
    pub fn search(
        &mut self,
        name: Option<&str>,
        pattern: &str,
    ) -> Result<Vec<SearchResult>, SessionError> {
        let (name, built) = self.ensure_index(name)?;
        let held = &self.vaults[&name];
        let lower = pattern.to_lowercase();
        let mut matched = held
            .index
            .as_ref()
            .expect("ensure_index built it")
            .match_text(&lower);
        let bodies = match built {
            Some(entries) => entries,
            None => held
                .vault
                .decrypt_all(held.identity.as_ref().expect("ensure_index unlocked it"))?,
        };
        for (id, e) in bodies {
            if !matched.contains_key(&id) && matches_body(&e, &lower) {
                matched.insert(
                    id,
                    SearchResult {
                        id,
                        title: e.title.clone(),
                        description: e.description.clone(),
                        matched_body: true,
                    },
                );
            }
        }
        let mut results: Vec<SearchResult> = matched.into_values().collect();
        sort_results(&mut results);
        Ok(results)
    }

    /// Discards a vault's index and builds it again from a full decrypt
    /// pass: for when `entries/` changed outside this session (a manual
    /// `git pull`), or the cache is merely suspect.
    pub fn reindex(&mut self, name: Option<&str>) -> Result<(), SessionError> {
        self.vault(name)?;
        let target = self.target(name)?;
        if let Some(held) = self.vaults.get_mut(&target) {
            discard_index(held);
        }
        self.ensure_index(Some(&target))?;
        Ok(())
    }

    /// Updates a vault's index with `e` after a write that has it in hand
    /// (`edit`). Cache bookkeeping only: the write itself was the
    /// `Vault` method's. A no-op for a vault with no index, whose eventual
    /// build will read the written state anyway.
    pub fn note_entry(&mut self, name: Option<&str>, id: Uuid, e: &Entry) {
        self.enter();
        let prompter = self.prompter;
        if let Some(held) = self.indexed(name) {
            held.index.as_mut().expect("indexed").put(id, e);
            warn_index_lock_once(held, prompter);
        }
    }

    /// Like [`Session::note_entry`] for a write that returned only an id
    /// (`insert`, `generate`, `rename`): reads that one entry, and only if
    /// the vault is indexed, so the cached dates are exactly the stored
    /// ones. It can't fail the write it follows: if the entry can't be
    /// read back, the index is dropped, and the next list rebuilds it and
    /// reports whatever is wrong.
    pub fn refresh_entry(&mut self, name: Option<&str>, id: Uuid) {
        self.enter();
        let prompter = self.prompter;
        let Some(held) = self.indexed(name) else {
            return;
        };
        let read = match &held.identity {
            Some(identity) => held.vault.read_entry(id, identity).ok(),
            None => None,
        };
        match read {
            Some(e) => {
                held.index.as_mut().expect("indexed").put(id, &e);
                warn_index_lock_once(held, prompter);
            }
            None => discard_index(held),
        }
    }

    /// Drops `id` from a vault's index after `rm`. A no-op with no index.
    pub fn forget_entry(&mut self, name: Option<&str>, id: Uuid) {
        self.enter();
        if let Some(held) = self.indexed(name) {
            held.index.as_mut().expect("indexed").remove(id);
        }
    }

    /// The current vault's entry titles, sorted, for tab completion of an
    /// entry query. Never unlocks and never prompts: no current vault, or
    /// one this session doesn't hold unlocked, offers nothing (completion
    /// has no way to ask a human, and must not turn a keystroke into a
    /// passphrase prompt). An unlocked vault without an index gets it
    /// built, once. Only the *current* vault: never one named by `--use`
    /// elsewhere on the line, and never another unlocked vault's titles.
    /// A failure to build the index offers nothing too; the command itself
    /// reports it when it runs.
    pub fn entry_title_candidates(&mut self) -> Vec<String> {
        self.enter();
        let prompter = self.prompter;
        let Some(held) = self
            .current
            .as_deref()
            .and_then(|name| self.vaults.get_mut(name))
            .filter(|h| h.identity.is_some())
        else {
            return Vec::new();
        };
        settle(held);
        if held.index.is_none() && build_index(held, prompter).is_err() {
            return Vec::new();
        }
        let index = held.index.as_ref().expect("built above");
        let mut titles: Vec<String> = index.titles().into_iter().map(|(_, t)| t).collect();
        titles.sort();
        titles.dedup();
        titles
    }

    /// Whether `name` currently has an index.
    #[cfg(test)]
    pub(crate) fn has_index(&self, name: &str) -> bool {
        self.vaults.get(name).is_some_and(|h| h.index.is_some())
    }

    /// A copy of `name`'s arena bytes, empty with no index.
    #[cfg(test)]
    pub(crate) fn index_arena(&self, name: &str) -> Vec<u8> {
        self.vaults
            .get(name)
            .and_then(|h| h.index.as_ref())
            .map(Index::arena_bytes)
            .unwrap_or_default()
    }

    /// Drops the key for `name`, or for every vault this session holds
    /// when `None`. Locked vaults stay listed by [`Session::status`]; the
    /// next call against one re-prompts.
    pub fn lock(&mut self, name: Option<&str>) -> Result<(), SessionError> {
        self.enter();
        match name {
            None => self.lock_all(),
            Some(name) => match self.vaults.get_mut(name) {
                Some(held) => lock_held(held),
                None => return Err(SessionError::NotHeld(name.to_string())),
            },
        }
        Ok(())
    }

    /// Every vault this session has touched, sorted by name. The idle
    /// timeout is applied first, so an aged-out vault reads as locked here
    /// without a command having to run: what keeps the prompt's lock glyph
    /// honest.
    pub fn status(&mut self) -> Vec<VaultState> {
        self.enter();
        let now = (self.now)();
        self.vaults
            .iter()
            .map(|(name, held)| VaultState {
                name: name.clone(),
                unlocked: held.identity.is_some(),
                current: self.current.as_deref() == Some(name),
                idle: held.last_used.map(|t| now.saturating_duration_since(t)),
            })
            .collect()
    }

    /// Drops every key this session holds: what the REPL does on `exit`
    /// and end of input. The same per-vault path [`Session::lock`] takes.
    /// Safe to repeat; dropping the session does the same.
    pub fn close(&mut self) {
        self.lock_all();
    }

    /// Marks the start of one command made of several session calls (the
    /// REPL brackets each typed line with it). The idle timeout is applied
    /// now, and then not again until the returned guard drops, so a key
    /// can't be dropped between a command's parts however long it runs.
    /// The guard derefs to the session.
    pub fn begin_command(&mut self) -> Command<'_, 'a> {
        self.enter();
        self.depth += 1;
        Command { session: self }
    }

    /// The idle timeout, applied on entering a call unless a command is
    /// in flight: a re-lock happens between operations, never inside one.
    fn enter(&mut self) {
        if self.depth > 0 {
            return;
        }
        let Some(timeout) = self.idle_timeout else {
            return;
        };
        let now = (self.now)();
        for held in self.vaults.values_mut() {
            let idle = held
                .last_used
                .is_some_and(|t| now.saturating_duration_since(t) > timeout);
            if held.identity.is_some() && idle {
                lock_held(held);
            }
        }
    }

    /// Unlocks a vault if needed and builds its index if it has none,
    /// returning its name and, when this call built the index, the
    /// entries decrypted to build it (so a first search reuses the pass).
    fn ensure_index(
        &mut self,
        name: Option<&str>,
    ) -> Result<(String, Option<Decrypted>), SessionError> {
        self.vault(name)?;
        let name = self.target(name)?;
        let prompter = self.prompter;
        let held = self.vaults.get_mut(&name).expect("vault() held it");
        if held.index.is_some() {
            return Ok((name, None));
        }
        let entries = build_index(held, prompter)?;
        Ok((name, Some(entries)))
    }

    /// The held vault `name` names (`None` is current), if it has an index.
    /// Never opens or unlocks anything.
    fn indexed(&mut self, name: Option<&str>) -> Option<&mut HeldVault> {
        let name = self.target(name).ok()?;
        let held = self.vaults.get_mut(&name)?;
        settle(held);
        held.index.is_some().then_some(held)
    }

    /// The vault a call operates on: the one named, else the current one.
    fn target(&self, name: Option<&str>) -> Result<String, SessionError> {
        name.filter(|n| !n.is_empty())
            .or(self.current.as_deref())
            .map(str::to_string)
            .ok_or(SessionError::NoCurrentVault {
                hint: "run `use <vault>` first, or pass -u/--use",
            })
    }

    /// This session's record for a vault, opened and registered on first
    /// sight, with its index dropped if a pull moved the tree since. Never
    /// unlocks.
    fn hold(&mut self, name: &str) -> Result<&mut HeldVault, SessionError> {
        if !self.vaults.contains_key(name) {
            // The vault is what knows when its tree moved, so it is told
            // whom to tell.
            let pulled = Arc::new(AtomicBool::new(false));
            let vault = (self.open)(name)?.with_pull_observer(Arc::clone(&pulled));
            self.vaults.insert(
                name.to_string(),
                HeldVault {
                    vault,
                    identity: None,
                    last_used: None,
                    index: None,
                    index_warned: false,
                    pulled,
                },
            );
        }
        let held = self.vaults.get_mut(name).expect("inserted above");
        settle(held);
        Ok(held)
    }

    fn lock_all(&mut self) {
        self.vaults.values_mut().for_each(lock_held);
    }
}

/// The one place a held vault's key is dropped, so an idle timeout, an
/// explicit lock and closing the session are the same code path. Dropping
/// the `Identity` is the whole teardown: its `Drop` wipes the key and
/// releases its page lock. The index goes first and with it: decrypted
/// metadata must not outlive the key that produced it.
fn lock_held(held: &mut HeldVault) {
    discard_index(held);
    held.identity = None;
}

/// Drops a vault's index (zeroed, its pages released). The one place that
/// happens, so locking, reindexing and a pull can't drift apart.
fn discard_index(held: &mut HeldVault) {
    held.index = None;
    held.index_warned = false;
}

/// Drops the index if the vault's tree moved since it was built, and
/// lowers the flag. Run before anything reads the index.
fn settle(held: &mut HeldVault) {
    if held.pulled.swap(false, Ordering::SeqCst) {
        discard_index(held);
    }
}

/// Builds `held`'s index from one decrypt pass over its (unlocked) vault,
/// and returns the entries decrypted to do it.
fn build_index(held: &mut HeldVault, prompter: &dyn Prompter) -> Result<Decrypted, SessionError> {
    let identity = held.identity.as_ref().expect("callers unlocked it");
    let entries = held.vault.decrypt_all(identity)?;
    let mut index = Index::new(held.vault.locker(), identity.page_locked());
    for (id, e) in &entries {
        index.put(*id, e);
    }
    held.index = Some(index);
    warn_index_lock_once(held, prompter);
    Ok(entries)
}

/// Reports, once per index, that its arena couldn't be page-locked. Never
/// when the vault's own key couldn't be either: the unlock already said
/// so (see `Index::lock_ok`).
fn warn_index_lock_once(held: &mut HeldVault, prompter: &dyn Prompter) {
    let failed = held.index.as_ref().is_some_and(|i| !i.lock_ok());
    if failed && !held.index_warned {
        held.index_warned = true;
        prompter.warn(&format!(
            "gage: could not lock {}'s metadata index into memory; continuing without it, \
             so cached titles/descriptions may be written to swap.",
            held.vault.name
        ));
    }
}

/// One command in flight; see [`Session::begin_command`].
pub struct Command<'s, 'a> {
    session: &'s mut Session<'a>,
}

impl<'a> std::ops::Deref for Command<'_, 'a> {
    type Target = Session<'a>;

    fn deref(&self) -> &Session<'a> {
        self.session
    }
}

impl<'a> std::ops::DerefMut for Command<'_, 'a> {
    fn deref_mut(&mut self) -> &mut Session<'a> {
        self.session
    }
}

impl Drop for Command<'_, '_> {
    fn drop(&mut self) {
        self.session.depth -= 1;
    }
}
