//! Golden Goal 02 capture-policy addresses retain their original inline
//! credential encoding after the classifier is shared with the Engine.
use celetixo_engine::ids::Digest;
use celetixo_repository::capture::{CapturePolicy, CredentialPolicy, policy_digest};

#[test]
fn capture_default_and_reviewed_policy_addresses_are_unchanged() {
    let mut policy = CapturePolicy::default();
    assert_eq!(
        policy_digest(&policy).to_string(),
        "99f4b07f42aac1d32a6407e0df9c19e62db8400e5cc2cb7ac42f24f6bc388fc9"
    );
    policy.credentials.reviewed_public_inputs.insert(
        ".env".into(),
        [Digest::of(
            "celetixo/source-content/v1",
            b"PUBLIC_API_BASE=https://example.invalid\n",
        )]
        .into(),
    );
    assert_eq!(
        policy_digest(&policy).to_string(),
        "822a3708e493d9a876edac423020ef2f754caab1d42cb3f85862fd00e1000eb4"
    );
    let _: celetixo_engine::credentials::CredentialPolicy = CredentialPolicy::default();
}
