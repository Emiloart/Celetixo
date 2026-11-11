//! Parse an exact public-input review explicitly supplied by a trusted operator.
//!
//! This does not inspect or classify content, discover policy in a repository, or
//! authorize a worker. The caller must independently review the selected bytes.
use celetixo_engine::credentials::{CredentialError, CredentialPolicy};
use celetixo_engine::ids::Digest;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ReviewArgumentError {
    #[error("public input review requires canonical PATH=64_LOWERCASE_HEX_CONTENT_DIGEST")]
    InvalidArgument,
    #[error("duplicate public input review")]
    Duplicate,
    #[error(transparent)]
    Policy(#[from] CredentialError),
}

/// Install the existing credential policy's exact path/content exception. The
/// digest uses `celetixo/source-content/v1`, not an unscoped raw SHA-256. No file
/// is read or automatically approved; malformed arguments leave policy unchanged.
pub fn add_exact_review(
    policy: &mut CredentialPolicy,
    argument: &str,
) -> Result<(), ReviewArgumentError> {
    if argument.len() > 4096 + 1 + 64 {
        return Err(ReviewArgumentError::InvalidArgument);
    }
    let (path, hex) = argument
        .split_once('=')
        .ok_or(ReviewArgumentError::InvalidArgument)?;
    if path.len() > 4096 || hex.len() != 64 {
        return Err(ReviewArgumentError::InvalidArgument);
    }
    let nibble = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ReviewArgumentError::InvalidArgument),
    };
    let mut bytes = [0; 32];
    for (target, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        *target = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    let mut proposed = policy.clone();
    if !proposed
        .reviewed_public_inputs
        .entry(path.into())
        .or_default()
        .insert(Digest::from_bytes(bytes))
    {
        return Err(ReviewArgumentError::Duplicate);
    }
    proposed.validate()?;
    *policy = proposed;
    Ok(())
}
