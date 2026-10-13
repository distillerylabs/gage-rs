//! The `gage` binary: the only place that touches the real standard
//! streams, asks whether stdin is a terminal, or exits the process.
//! Everything else is `gage_cli::run`.

use std::io::IsTerminal;

use gage::xdgpaths::Dirs;
use gage_cli::{BuildInfo, Io};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdin = std::io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    let mut io = Io {
        stdin: &mut stdin.lock(),
        stdout: &mut std::io::stdout().lock(),
        stderr: &mut std::io::stderr().lock(),
        stdin_is_terminal,
        build: BuildInfo::stamped(),
        dirs: &Dirs::from_process,
        hostname: &|| Ok(hostname::get()?.to_string_lossy().into_owned()),
    };
    let code = gage_cli::run(&args, &mut io, &NoPrompter);
    std::process::exit(code.as_i32());
}

/// Nothing asks a question yet; the terminal prompter arrives with the
/// first command that needs one (M2's passphrase).
struct NoPrompter;

impl gage::prompter::Prompter for NoPrompter {
    fn unlock(
        &self,
        _req: &gage::prompter::UnlockRequest,
    ) -> Result<gage::prompter::UnlockResponse, gage::prompter::PromptError> {
        Err(gage::prompter::PromptError::Cancelled)
    }

    fn confirm(&self, _prompt: &str) -> Result<bool, gage::prompter::PromptError> {
        Err(gage::prompter::PromptError::Cancelled)
    }
}
