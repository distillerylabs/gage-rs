use super::*;

fn login() -> Remedy {
    Remedy::AuthLogin {
        host: "github.com".into(),
    }
}

#[test]
fn only_the_remedies_that_let_a_remote_in_are_access_remedies() {
    for access in [
        login(),
        Remedy::AuthLoginAgain {
            host: "github.com".into(),
        },
        Remedy::UseHttps,
    ] {
        assert!(access.fixes_remote_access(), "{access:?}");
    }
    for other in [
        Remedy::Sync,
        Remedy::Push,
        Remedy::EnrollAgain,
        Remedy::SetRemote,
    ] {
        assert!(!other.fixes_remote_access(), "{other:?}");
    }
}

// ADR-0079: whatever the wrapper would say instead, a refusal of access
// keeps the remedy that ends it.
#[test]
fn a_wrappers_remedy_never_shadows_an_access_remedy() {
    assert_eq!(
        Remedy::unless_access(Some(login()), Some(Remedy::Push)),
        Some(login())
    );
    assert_eq!(
        Remedy::unless_access(Some(Remedy::UseHttps), Some(Remedy::EnrollAgain)),
        Some(Remedy::UseHttps)
    );
}

#[test]
fn otherwise_the_wrappers_remedy_wins_and_the_cause_is_the_fallback() {
    assert_eq!(
        Remedy::unless_access(Some(Remedy::Sync), Some(Remedy::Push)),
        Some(Remedy::Push)
    );
    assert_eq!(
        Remedy::unless_access(Some(Remedy::Sync), None),
        Some(Remedy::Sync)
    );
    assert_eq!(Remedy::unless_access(None, None), None);
}
