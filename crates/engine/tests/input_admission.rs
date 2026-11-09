//! Fixed heuristic credential boundaries; these are controlled public fixtures.
use celetixo_engine::credentials::*;
use celetixo_engine::ids::Digest;

#[test]
fn reviewed_public_exceptions_require_exact_path_and_exact_content() {
    let content = b"PUBLIC_API_BASE=https://example.invalid\n";
    let mut policy = CredentialPolicy::default();
    assert!(matches!(
        policy.admit_input(".env", content),
        Err(CredentialError::SensitiveInput(_))
    ));
    policy.reviewed_public_inputs.insert(
        ".env".into(),
        [Digest::of("celetixo/source-content/v1", content)].into(),
    );
    policy.admit_input(".env", content).unwrap();
    assert!(
        policy
            .admit_input(".env", b"PUBLIC_API_BASE=https://changed.invalid\n")
            .is_err()
    );
    assert!(policy.admit_input("other/.env", content).is_err());
    // Public template paths do not bypass content-pattern admission.
    let provider = format!("{}{}", "ghp_", "x".repeat(36));
    assert!(
        policy
            .admit_input(".env.example", provider.as_bytes())
            .is_err()
    );
}

#[test]
fn malformed_policy_cannot_authorize_an_input() {
    let mut policy = CredentialPolicy {
        version: 2,
        ..Default::default()
    };
    assert_eq!(
        policy.admit_input("main.rs", b"public"),
        Err(CredentialError::UnsupportedPolicy)
    );
    policy.version = 1;
    policy
        .reviewed_public_inputs
        .insert("../escape".into(), [Digest::of("test", b"public")].into());
    assert_eq!(policy.validate(), Err(CredentialError::InvalidReviewPath));
    policy.reviewed_public_inputs.clear();
    policy
        .reviewed_public_inputs
        .insert("public.txt".into(), Default::default());
    assert_eq!(policy.validate(), Err(CredentialError::EmptyReview));
}

#[test]
fn finite_patterns_cover_binary_provider_material_keys_and_credential_urls() {
    let provider = format!("{}{}", "ghp_", "x".repeat(36));
    let private_key = format!(
        "-----BEGIN {}-----\n{}\n-----END {}-----",
        "PRIVATE KEY",
        "A".repeat(64),
        "PRIVATE KEY"
    );
    let database = [
        "postgres://user:",
        "realpassword",
        "@example.invalid/database",
    ]
    .concat();
    let mut binary = vec![0xff, 0];
    binary.extend_from_slice(provider.as_bytes());
    let policy = CredentialPolicy::default();
    for bytes in [
        provider.as_bytes(),
        private_key.as_bytes(),
        database.as_bytes(),
        binary.as_slice(),
    ] {
        assert!(credential_material(bytes));
        let error = policy.admit_input("ordinary.data", bytes).unwrap_err();
        assert!(
            !error
                .to_string()
                .contains(&String::from_utf8_lossy(bytes).to_string())
        );
    }
    assert!(credential_key("AWS_ACCESS_KEY"));
    assert!(credential_key("database.password"));
    assert!(!credential_key("monkey"));
}

#[test]
fn existing_program_variables_and_public_placeholders_remain_accepted() {
    let source = b"let key = lookup();\nlet token = get_token();\npassword = '<placeholder>'\napi_key = process.env.API_KEY\n";
    assert!(!credential_material(source));
    CredentialPolicy::default()
        .admit_input("source.rs", source)
        .unwrap();
    assert!(!credential_path(".env.example"));
    assert!(credential_path("nested/.aws/credentials"));
}

#[test]
fn policy_identity_binds_reviewed_inputs_and_is_delivery_order_independent() {
    let mut first = CredentialPolicy::default();
    let base = first.digest();
    let a = Digest::of("celetixo/source-content/v1", b"a");
    let b = Digest::of("celetixo/source-content/v1", b"b");
    first
        .reviewed_public_inputs
        .insert("a.env".into(), [a, b].into());
    first
        .reviewed_public_inputs
        .insert("b.env".into(), [b].into());
    let mut second = CredentialPolicy::default();
    second
        .reviewed_public_inputs
        .insert("b.env".into(), [b].into());
    second
        .reviewed_public_inputs
        .insert("a.env".into(), [b, a].into());
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.digest(), second.digest());
    assert_ne!(base, first.digest());
    second
        .reviewed_public_inputs
        .get_mut("a.env")
        .unwrap()
        .remove(&b);
    assert_ne!(first.digest(), second.digest());
}
