use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::ids::Digest;
use celetixo_repository::reviewed_inputs::{ReviewArgumentError, add_exact_review};

fn review(path: &str, bytes: &[u8]) -> String {
    format!("{path}={}", Digest::of("celetixo/source-content/v1", bytes))
}

#[test]
fn explicit_review_admits_only_the_selected_public_path_and_exact_bytes() {
    let bytes = b"password_encryption = 'scram-sha-256'";
    let mut policy = CredentialPolicy::default();
    assert!(policy.admit_input("public.conf", bytes).is_err());
    add_exact_review(&mut policy, &review("public.conf", bytes)).unwrap();
    policy.admit_input("public.conf", bytes).unwrap();
    assert!(policy.admit_input("elsewhere.conf", bytes).is_err());
    let changed = [bytes.as_slice(), b"\n"].concat();
    assert!(policy.admit_input("public.conf", &changed).is_err());
    assert_ne!(policy.digest(), CredentialPolicy::default().digest());
}

#[test]
fn malformed_digest_and_noncanonical_paths_do_not_partially_mutate_policy() {
    let mut policy = CredentialPolicy::default();
    for argument in [
        "missing-digest".into(),
        "public.conf=abcd".into(),
        format!("public.conf={}", "G".repeat(64)),
        review("../outside", b"public"),
        review("/absolute", b"public"),
        review("a/./b", b"public"),
        review("a\\b", b"public"),
        review("", b"public"),
        review(&"a".repeat(4097), b"public"),
    ] {
        assert!(add_exact_review(&mut policy, &argument).is_err());
        assert_eq!(policy, CredentialPolicy::default());
    }
}

#[test]
fn distinct_reviewed_versions_survive_but_duplicates_and_unknown_policy_reject() {
    let mut policy = CredentialPolicy::default();
    let first = review("public.conf", b"first public version");
    add_exact_review(&mut policy, &first).unwrap();
    let saved = policy.clone();
    assert_eq!(
        add_exact_review(&mut policy, &first),
        Err(ReviewArgumentError::Duplicate)
    );
    assert_eq!(policy, saved);
    add_exact_review(
        &mut policy,
        &review("public.conf", b"second public version"),
    )
    .unwrap();
    assert_eq!(policy.reviewed_public_inputs["public.conf"].len(), 2);
    policy.version = 99;
    let saved = policy.clone();
    assert!(add_exact_review(&mut policy, &review("another.conf", b"public")).is_err());
    assert_eq!(policy, saved);
}
