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
//! Go holds `*Identity` pointers and needs a closed-identity contract so a
//! re-lock can't pull a key out from under a command still using it. Here
//! a command's `&Identity` borrows the session, and every way to lock
//! needs `&mut Session`, so that overlap doesn't compile (Q-SESSION-SHAPE).
//! What survives of Go's call-depth rule is *when* the idle timeout runs:
//! on entering a call, never between the calls of one command.
//!
//! No metadata index yet: a session still decrypts on demand (M7).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::entry::{self, Entry, ResolveError};
use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;
use crate::identityfile::UnlockError;
use crate::prompter::{PromptError, Prompter};
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
            current: cfg.current.filter(|c| !c.is_empty()),
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
    pub fn use_vault(&mut self, name: &str) -> Result<(), SessionError> {
        if name.is_empty() {
            return Err(SessionError::NoCurrentVault {
                hint: "use <vault>",
            });
        }
        self.vault(Some(name))?;
        self.current = Some(name.to_string());
        Ok(())
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
        let (vault, identity) = self.vault(name)?;
        let list = match vault.resolve(identity, query) {
            Err(ResolveError::Ambiguous(list)) => list,
            other => return Ok(other?),
        };
        let chosen = match prompter.choose(&list) {
            Ok(id) => id,
            Err(PromptError::Cancelled) => return Err(ResolveError::Ambiguous(list).into()),
            Err(e) => return Err(SessionError::Choose(e)),
        };
        if !list.candidates.iter().any(|c| c.id == chosen) {
            return Err(SessionError::NotACandidate(chosen));
        }
        Ok((chosen, vault.read_entry(chosen, identity)?))
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
    /// sight. Never unlocks.
    fn hold(&mut self, name: &str) -> Result<&mut HeldVault, SessionError> {
        if !self.vaults.contains_key(name) {
            let vault = (self.open)(name)?;
            self.vaults.insert(
                name.to_string(),
                HeldVault {
                    vault,
                    identity: None,
                    last_used: None,
                },
            );
        }
        Ok(self.vaults.get_mut(name).expect("inserted above"))
    }

    fn lock_all(&mut self) {
        self.vaults.values_mut().for_each(lock_held);
    }
}

/// The one place a held vault's key is dropped, so an idle timeout, an
/// explicit lock and closing the session are the same code path. Dropping
/// the `Identity` is the whole teardown: its `Drop` wipes the key and
/// releases its page lock.
fn lock_held(held: &mut HeldVault) {
    held.identity = None;
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
