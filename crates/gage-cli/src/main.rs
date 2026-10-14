//! The `gage` binary: the only place that touches the real standard
//! streams, asks whether stdin is a terminal, or exits the process.
//! Everything else is `gage_cli::run`.

use std::io::IsTerminal;

use gage::xdgpaths::Dirs;
use gage_cli::prompter::{SecretInput, TerminalPrompter};
use gage_cli::{BuildInfo, Io};

fn main() {
    // Before anything else, and before any key material can exist. A
    // failure is reported and the program continues, the same
    // warn-and-proceed posture page-locking takes: a weaker guarantee
    // beats an unusable tool.
    if let Err(e) = gage_cli::coredump::disable_core_dumps() {
        eprintln!("gage: could not disable core dumps: {e}");
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let stdin_is_terminal = std::io::stdin().is_terminal();
    // Prompts go to stderr, so `gage show foo > secret.txt` gets only the
    // secret. Neither side holds stdin's lock for the whole run: the
    // prompter and a command reading its input share the one stream, and
    // that lock isn't reentrant.
    let prompter = TerminalPrompter::new(
        std::io::stdin(),
        std::io::stderr(),
        if stdin_is_terminal {
            SecretInput::Terminal(None)
        } else {
            SecretInput::Lines
        },
    );
    let mut io = Io {
        stdin: &mut std::io::stdin(),
        stdout: &mut std::io::stdout().lock(),
        stderr: &mut std::io::stderr(),
        stdin_is_terminal,
        build: BuildInfo::stamped(),
        dirs: &Dirs::from_process,
        hostname: &|| Ok(hostname::get()?.to_string_lossy().into_owned()),
    };
    let code = gage_cli::run(&args, &mut io, &prompter);
    std::process::exit(code.as_i32());
}
