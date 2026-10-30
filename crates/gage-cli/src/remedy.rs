//! A library [`Remedy`] as the command line names it: the clause gage
//! appends to an error or warning, separator included (ADR-0054). The
//! library says what would fix it; only this module spells it as a `gage`
//! command.

use gage::Remedy;
use gage::Warning;

use crate::error::ERROR_PREFIX;

/// The clause that follows a message offering `remedy`.
pub fn render(remedy: &Remedy) -> String {
    match remedy {
        Remedy::Sync => "; run `gage sync`".into(),
        Remedy::SyncAgain => "; run `gage sync` again to finish".into(),
        Remedy::RecipientVerify => "; run `gage recipient verify`".into(),
        Remedy::RecipientRepair => format!(
            "\n{ERROR_PREFIX}fix this with `gage recipient verify --repair`, which rewrites \
             .age-recipients from .gage/config.toml and says what it drops"
        ),
        Remedy::AuthLogin { host } => format!(
            "; run `gage auth login --host {host}` to store a token for {host} \
             (an existing one may have expired or been revoked)"
        ),
        Remedy::AuthLoginAgain { host } => {
            format!("; fix it or run `gage auth login --host {host}` again")
        }
        Remedy::UseHttps => "; use the remote's HTTPS URL with `gage auth login` instead".into(),
        Remedy::ReRegister { name } => {
            format!("; re-register it with `gage vault remove {name}` and `gage clone`")
        }
        Remedy::EnrollOrRecover => "; if this device has none yet, run `gage identity enroll` \
                                    to ask to join this vault, or `gage recovery enroll` if you \
                                    hold its recovery key"
            .into(),
        Remedy::SetRemote => "; configure one with `gage git set-remote`".into(),
        Remedy::Reclone { vault } => format!(
            ", so discard this copy (`gage vault remove {vault}`), `gage clone` it again, \
             then run `gage identity enroll`"
        ),
        Remedy::EnrollAgain {
            auth_refused: false,
            ..
        } => ", so running `gage identity enroll` again is the way forward".into(),
        Remedy::EnrollAgain {
            auth_refused: true,
            host: Some(host),
        } => format!(
            "; run `gage auth login --host {host}` first, then `gage identity enroll` again"
        ),
        Remedy::EnrollAgain {
            auth_refused: true,
            host: None,
        } => "; run `gage auth login` first, then `gage identity enroll` again".into(),
        Remedy::Push => " — run `gage push` to finish it".into(),
        Remedy::Force => "; use --force to insert a duplicate".into(),
        Remedy::ChooseDevice => "; choose another with --device".into(),
        Remedy::ApproveWithDevice => {
            "; approve with --device NAME to record this request under a different name".into()
        }
        Remedy::PassName { plain: false } => "; pass --name".into(),
        Remedy::PassName { plain: true } => "; pass --name with a plain name".into(),
        Remedy::Yes => ". Run the command where you can answer, or with --yes".into(),
        // A remedy this build doesn't know how to name: the message stands
        // on its own.
        _ => String::new(),
    }
}

/// A warning as stderr shows it: gage's name, the text, then the remedy.
pub fn render_warning(warning: &Warning) -> String {
    let remedy = warning.remedy.as_ref().map(render).unwrap_or_default();
    format!("{ERROR_PREFIX}{}{remedy}", warning.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each rendering is the hint text the library carried before the
    // command text moved here (D3b decision 2's table), byte for byte.
    #[test]
    fn every_remedy_renders_its_hint() {
        let host = || "github.com".to_string();
        let cases: Vec<(Remedy, &str)> = vec![
            (Remedy::Sync, "; run `gage sync`"),
            (Remedy::SyncAgain, "; run `gage sync` again to finish"),
            (Remedy::RecipientVerify, "; run `gage recipient verify`"),
            (
                Remedy::RecipientRepair,
                "\ngage: fix this with `gage recipient verify --repair`, which rewrites .age-recipients from .gage/config.toml and says what it drops",
            ),
            (
                Remedy::AuthLogin { host: host() },
                "; run `gage auth login --host github.com` to store a token for github.com (an existing one may have expired or been revoked)",
            ),
            (
                Remedy::AuthLoginAgain { host: host() },
                "; fix it or run `gage auth login --host github.com` again",
            ),
            (
                Remedy::UseHttps,
                "; use the remote's HTTPS URL with `gage auth login` instead",
            ),
            (
                Remedy::ReRegister {
                    name: "work".into(),
                },
                "; re-register it with `gage vault remove work` and `gage clone`",
            ),
            (
                Remedy::EnrollOrRecover,
                "; if this device has none yet, run `gage identity enroll` to ask to join this vault, or `gage recovery enroll` if you hold its recovery key",
            ),
            (
                Remedy::SetRemote,
                "; configure one with `gage git set-remote`",
            ),
            (
                Remedy::Reclone {
                    vault: "work".into(),
                },
                ", so discard this copy (`gage vault remove work`), `gage clone` it again, then run `gage identity enroll`",
            ),
            (
                Remedy::EnrollAgain {
                    auth_refused: false,
                    host: None,
                },
                ", so running `gage identity enroll` again is the way forward",
            ),
            (
                Remedy::EnrollAgain {
                    auth_refused: true,
                    host: Some(host()),
                },
                "; run `gage auth login --host github.com` first, then `gage identity enroll` again",
            ),
            (
                Remedy::EnrollAgain {
                    auth_refused: true,
                    host: None,
                },
                "; run `gage auth login` first, then `gage identity enroll` again",
            ),
            (Remedy::Push, " — run `gage push` to finish it"),
            (Remedy::Force, "; use --force to insert a duplicate"),
            (Remedy::ChooseDevice, "; choose another with --device"),
            (
                Remedy::ApproveWithDevice,
                "; approve with --device NAME to record this request under a different name",
            ),
            (Remedy::PassName { plain: false }, "; pass --name"),
            (
                Remedy::PassName { plain: true },
                "; pass --name with a plain name",
            ),
            (
                Remedy::Yes,
                ". Run the command where you can answer, or with --yes",
            ),
        ];
        for (remedy, want) in cases {
            assert_eq!(render(&remedy), want, "{remedy:?}");
        }
    }

    #[test]
    fn a_warning_gets_gages_name_once_and_its_remedy_last() {
        assert_eq!(render_warning(&Warning::from("careful")), "gage: careful");
        assert_eq!(
            render_warning(&Warning::from("diverged").with_remedy(Some(Remedy::Sync))),
            "gage: diverged; run `gage sync`"
        );
    }
}
