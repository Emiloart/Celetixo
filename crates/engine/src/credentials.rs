//! Fixed, versioned credential admission shared by capture and source execution.
//!
//! These conservative finite rules can miss unknown, encoded or fragmented
//! secrets and can reject public examples. They are not a complete data
//! classification service. Only trusted installed policy may authorize an exact
//! reviewed path/content exception; diagnostics never retain rejected bytes.

use crate::ids::{Canonical, Digest};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialPolicy {
    /// Version of the fixed admission rules. Unsupported versions reject.
    pub version: u32,
    /// Reviewed public content, keyed by exact logical path and content digest.
    /// This policy must be supplied by the trusted capture/governance caller;
    /// untrusted workers cannot authorize their own exceptions. A path-only or
    /// wildcard exception is deliberately unavailable.
    pub reviewed_public_inputs: BTreeMap<String, BTreeSet<Digest>>,
}

impl Default for CredentialPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            reviewed_public_inputs: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialError {
    #[error("unsupported credential admission policy version")]
    UnsupportedPolicy,
    #[error("reviewed public input path is not canonical")]
    InvalidReviewPath,
    #[error("reviewed public input needs at least one exact content digest")]
    EmptyReview,
    #[error(
        "input admission rejected potential credential content at logical path {0:?}; content withheld"
    )]
    SensitiveInput(String),
}

impl CredentialPolicy {
    pub fn validate(&self) -> Result<(), CredentialError> {
        if self.version != 1 {
            return Err(CredentialError::UnsupportedPolicy);
        }
        for (path, digests) in &self.reviewed_public_inputs {
            if path.is_empty()
                || path.contains(['\\', ':'])
                || path.chars().any(char::is_control)
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(CredentialError::InvalidReviewPath);
            }
            if digests.is_empty() {
                return Err(CredentialError::EmptyReview);
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.u64(u64::from(self.version));
        c.u64(self.reviewed_public_inputs.len() as u64);
        for (path, digests) in &self.reviewed_public_inputs {
            c.str(path);
            c.u64(digests.len() as u64);
            for digest in digests {
                c.digest(*digest);
            }
        }
        c.into_bytes()
    }

    pub fn digest(&self) -> Digest {
        Digest::of(
            "celetixo/credential-admission-policy/v1",
            &self.canonical_bytes(),
        )
    }

    pub fn admit_input(&self, path: &str, bytes: &[u8]) -> Result<(), CredentialError> {
        admit_input(path, bytes, self)
    }
}

pub fn admit_input(
    path: &str,
    bytes: &[u8],
    policy: &CredentialPolicy,
) -> Result<(), CredentialError> {
    policy.validate()?;
    if !credential_path(path) && !credential_material(bytes) {
        return Ok(());
    }
    let digest = Digest::of("celetixo/source-content/v1", bytes);
    if policy
        .reviewed_public_inputs
        .get(path)
        .is_some_and(|digests| digests.contains(&digest))
    {
        return Ok(());
    }
    Err(CredentialError::SensitiveInput(path.into()))
}

pub fn credential_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let public_template = [".example", ".sample", ".template"]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    if (name == ".env" || name.starts_with(".env.")) && !public_template {
        return true;
    }
    if matches!(
        name,
        ".git-credentials"
            | ".netrc"
            | "_netrc"
            | ".pypirc"
            | "id_rsa"
            | "id_dsa"
            | "id_ecdsa"
            | "id_ed25519"
            | "credentials"
            | "access_token"
            | "refresh_token"
            | "token"
    ) {
        return true;
    }
    if [".key", ".p12", ".pfx", ".ppk", ".p8", ".jks", ".keystore"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return true;
    }
    if ["credentials.", "secrets.", "tokens."]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        && !public_template
    {
        return true;
    }
    [
        ".aws/credentials",
        ".azure/accesstokens.json",
        ".docker/config.json",
    ]
    .iter()
    .any(|suffix| lower == *suffix || lower.ends_with(&format!("/{suffix}")))
}

pub fn credential_material(bytes: &[u8]) -> bool {
    let token = |prefix: &[u8], minimum: usize, valid: fn(u8) -> bool| {
        bytes
            .windows(prefix.len())
            .enumerate()
            .any(|(offset, window)| {
                window == prefix
                    && bytes
                        .get(offset + prefix.len()..offset + prefix.len() + minimum)
                        .is_some_and(|suffix| suffix.iter().all(|byte| valid(*byte)))
            })
    };
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-');
    for (prefix, minimum) in [
        (b"ghp_".as_slice(), 36),
        (b"gho_", 36),
        (b"ghu_", 36),
        (b"ghs_", 36),
        (b"ghr_", 36),
        (b"github_pat_", 30),
        (b"xoxb-", 20),
        (b"xoxp-", 20),
        (b"xoxa-", 20),
        (b"sk-proj-", 20),
        (b"sk-", 30),
        (b"AIza", 35),
    ] {
        if token(prefix, minimum, identifier) {
            return true;
        }
    }
    for prefix in [b"AKIA", b"ASIA"] {
        if token(prefix, 16, |byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit()
        }) {
            return true;
        }
    }
    if bytes
        .windows(b"openssh-key-v1\0".len())
        .any(|window| window == b"openssh-key-v1\0")
    {
        return true;
    }
    for header in [
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN EC PRIVATE KEY-----",
        "-----BEGIN DSA PRIVATE KEY-----",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----BEGIN ENCRYPTED PRIVATE KEY-----",
    ] {
        for (offset, _) in bytes
            .windows(header.len())
            .enumerate()
            .filter(|(_, window)| *window == header.as_bytes())
        {
            let following = &bytes[offset + header.len()..];
            let following = following
                .strip_prefix(b"\r\n")
                .or_else(|| following.strip_prefix(b"\n"))
                .or_else(|| following.strip_prefix(b"\\n"));
            if following.is_some_and(|payload| {
                payload.get(..32).is_some_and(|payload| {
                    payload.iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                    })
                })
            }) {
                return true;
            }
        }
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    for (offset, _) in text.match_indices("://") {
        let authority = text[offset + 3..]
            .split(['/', ' ', '\t', '\r', '\n', '"', '\''])
            .next()
            .unwrap_or("");
        if let Some((user_info, _)) = authority.rsplit_once('@')
            && let Some((_, password)) = user_info.split_once(':')
            && !password.is_empty()
            && !public_placeholder(password)
        {
            return true;
        }
    }
    for line in text.lines() {
        let Some(delimiter) = line.find(['=', ':']) else {
            continue;
        };
        let (key, value) = (&line[..delimiter], &line[delimiter + 1..]);
        let declared_variable = key.split_whitespace().count() > 1;
        let quoted_value = value.trim().starts_with(['\'', '"']);
        if declared_variable && !quoted_value {
            continue;
        }
        let key = key.trim().trim_matches(['\'', '"']).to_ascii_lowercase();
        let key = key.split_whitespace().last().unwrap_or("");
        if !credential_key(key) {
            continue;
        }
        let value = value.trim().trim_end_matches(',').trim_matches(['\'', '"']);
        if !public_placeholder(value) && value.len() >= 8 {
            return true;
        }
    }
    false
}

pub fn credential_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let parts = lower.split(['_', '-', '.']).collect::<Vec<_>>();
    parts.iter().any(|part| {
        matches!(
            *part,
            "password" | "passwd" | "token" | "secret" | "apikey" | "authtoken"
        )
    }) || parts.windows(2).any(|parts| {
        matches!(
            parts,
            ["api", "key"] | ["private", "key"] | ["access", "key"]
        )
    })
}

fn public_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.is_empty()
        || value.starts_with(['<', '$'])
        || [
            "example",
            "placeholder",
            "dummy",
            "fake",
            "not-a-secret",
            "redacted",
            "changeme",
            "change-me",
            "your_",
            "your-",
            "process.env",
            "os.environ",
            "std::env",
            "env::",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || matches!(
            lower.as_str(),
            "null" | "none" | "undefined" | "true" | "false" | "..."
        )
}
