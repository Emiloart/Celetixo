//! Immutable source artifact/receipt metadata, not current object availability.
use super::*;
use crate::engine::SourceEvidenceReceipt;
use crate::ids::{Canonical, CheckId, SourceAdmissionId, SourceEvidenceId};
use crate::source_evidence::{SourceArtifactReference, SourceEvidenceAssessment};

/// A logical reference is distinct from physical bytes and from the same bytes
/// retained through another configured registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceArtifactReferenceRecord {
    scope: Scope,
    registration: u64,
    reference: SourceArtifactReference,
}
impl SourceArtifactReferenceRecord {
    pub const FORMAT: &str = "celetixo/source-artifact-reference-record/v1";
    pub fn new(
        scope: Scope,
        registration: u64,
        reference: SourceArtifactReference,
    ) -> Result<Self, RestoreError> {
        if registration == 0 || reference.scope != scope {
            return Err(RestoreError::BindingMismatch);
        }
        for (text, limit) in [(&reference.name, 4096), (&reference.media_type, 1024)] {
            if text.trim().is_empty() || text.len() > limit || text.chars().any(char::is_control) {
                return Err(RestoreError::InvalidDomainValue);
            }
        }
        Ok(Self {
            scope,
            registration,
            reference,
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn registration(&self) -> u64 {
        self.registration
    }
    pub fn reference(&self) -> &SourceArtifactReference {
        &self.reference
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        encode_scope(&mut c, &self.scope);
        c.u64(self.registration);
        c.str(&self.reference.name);
        c.str(&self.reference.media_type);
        c.digest(self.reference.content);
        c.u64(self.reference.bytes);
        c.digest(self.reference.retention_policy);
        c.into_bytes()
    }
}
pub fn restore_source_artifact_reference(
    scope: &Scope,
    registration: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceArtifactReferenceRecord, RestoreError> {
    let mut r = Reader::new(
        expected,
        SourceArtifactReferenceRecord::FORMAT,
        bytes,
        limits,
    )?;
    if r.scope()? != *scope || r.number()? != registration {
        return Err(RestoreError::BindingMismatch);
    }
    let reference = SourceArtifactReference {
        scope: scope.clone(),
        name: r.text()?,
        media_type: r.text()?,
        content: r.digest()?,
        bytes: r.number()?,
        retention_policy: r.digest()?,
    };
    r.end()?;
    let record = SourceArtifactReferenceRecord::new(scope.clone(), registration, reference)?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidenceReceiptRecord {
    scope: Scope,
    registration: u64,
    receipt: SourceEvidenceReceipt,
    envelope: SourceArtifactReferenceRecord,
}
impl SourceEvidenceReceiptRecord {
    pub const FORMAT: &str = "celetixo/source-evidence-receipt/v1";
    pub fn new(
        scope: Scope,
        registration: u64,
        receipt: SourceEvidenceReceipt,
    ) -> Result<Self, RestoreError> {
        match &receipt.assessment {
            SourceEvidenceAssessment::Failed { checks } if checks.is_empty() => {
                return Err(RestoreError::InvalidDomainValue);
            }
            SourceEvidenceAssessment::Incomplete { reasons }
                if reasons.is_empty() || reasons.iter().any(|r| r.trim().is_empty()) =>
            {
                return Err(RestoreError::InvalidDomainValue);
            }
            _ => {}
        }
        let envelope = SourceArtifactReferenceRecord::new(
            scope.clone(),
            registration,
            receipt.envelope.clone(),
        )?;
        Ok(Self {
            scope,
            registration,
            receipt,
            envelope,
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn registration(&self) -> u64 {
        self.registration
    }
    pub fn receipt(&self) -> &SourceEvidenceReceipt {
        &self.receipt
    }
    pub fn envelope(&self) -> &SourceArtifactReferenceRecord {
        &self.envelope
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        encode_scope(&mut c, &self.scope);
        c.u64(self.registration);
        c.digest(self.receipt.admission.0);
        c.digest(self.receipt.evidence.0);
        match &self.receipt.assessment {
            SourceEvidenceAssessment::Passed => c.str("Passed"),
            SourceEvidenceAssessment::Failed { checks } => {
                c.str("Failed");
                c.u64(checks.len() as u64);
                for check in checks {
                    c.str(check.as_str());
                }
            }
            SourceEvidenceAssessment::Incomplete { reasons } => {
                c.str("Incomplete");
                c.u64(reasons.len() as u64);
                for reason in reasons {
                    c.str(reason);
                }
            }
        }
        c.u64(self.receipt.recorded_at_ms);
        c.bytes(&self.envelope.canonical_bytes());
        c.digest(self.receipt.storage_provenance);
        c.into_bytes()
    }
}
pub fn restore_source_evidence_receipt(
    scope: &Scope,
    registration: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceEvidenceReceiptRecord, RestoreError> {
    let mut r = Reader::new(expected, SourceEvidenceReceiptRecord::FORMAT, bytes, limits)?;
    if r.scope()? != *scope || r.number()? != registration {
        return Err(RestoreError::BindingMismatch);
    }
    let admission = SourceAdmissionId(r.digest()?);
    let evidence = SourceEvidenceId(r.digest()?);
    let assessment = match r.text()?.as_str() {
        "Passed" => SourceEvidenceAssessment::Passed,
        "Failed" => {
            let mut checks = BTreeSet::new();
            for _ in 0..r.count()? {
                if !checks.insert(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?) {
                    return Err(RestoreError::NonCanonical);
                }
            }
            SourceEvidenceAssessment::Failed { checks }
        }
        "Incomplete" => {
            let mut reasons = BTreeSet::new();
            for _ in 0..r.count()? {
                if !reasons.insert(r.text()?) {
                    return Err(RestoreError::NonCanonical);
                }
            }
            SourceEvidenceAssessment::Incomplete { reasons }
        }
        _ => return Err(RestoreError::Encoding),
    };
    let recorded_at_ms = r.number()?;
    let envelope_bytes = r.field()?;
    let envelope = restore_source_artifact_reference(
        scope,
        registration,
        Digest::of(SourceArtifactReferenceRecord::FORMAT, envelope_bytes),
        envelope_bytes,
        limits,
    )?;
    let storage_provenance = r.digest()?;
    r.end()?;
    let record = SourceEvidenceReceiptRecord::new(
        scope.clone(),
        registration,
        SourceEvidenceReceipt {
            admission,
            evidence,
            assessment,
            recorded_at_ms,
            envelope: envelope.reference().clone(),
            storage_provenance,
        },
    )?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}
fn encode_scope(c: &mut Canonical, scope: &Scope) {
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
}
