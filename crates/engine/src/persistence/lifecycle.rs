//! Checked immutable values needed to reconstruct normalized lifecycle rows.
//! Native reference intent records are not the cross-service TaskIntent DSL.
//! These values do not restore capabilities or confer verification/commit authority.
use super::*;
use crate::access::{
    AccessSet, AnalysisStamp, Completeness, Condition, ImpactAnalysis, Invariant, Predicate,
};
use crate::engine::CommitRequest;
use crate::evidence::{
    Artifact, CheckResult, CheckSpec, EvidenceBindings, ExecutionProvenance, VerificationEvidence,
};
use crate::ids::{
    Canonical, CheckId, EvidenceId, IdempotencyKey, InvariantId, PatchId, PrincipalId,
    TransactionId,
};
use crate::patch::{Change, Patch};
use crate::transaction::TransactionIntent;

/// Restore the original immutable request, including its old evidence set. The
/// key is an external lookup identity and deliberately absent from the semantic
/// digest. Neither the body nor its grant ID authenticates the caller.
pub fn restore_commit_request(
    scope: &Scope,
    principal: &PrincipalId,
    key: IdempotencyKey,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<CommitRequest, RestoreError> {
    let mut r = Reader::new(expected, CommitRequest::FORMAT, bytes, limits)?;
    if r.scope()? != *scope || r.text()? != principal.as_str() {
        return Err(RestoreError::BindingMismatch);
    }
    let transaction = TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let attempt = r.number()?;
    let authorization_grant = r.number()?;
    let fence = r.number()?;
    if authorization_grant == 0 || fence == 0 {
        return Err(RestoreError::InvalidDomainValue);
    }
    let base_snapshot = SnapshotId(r.digest()?);
    let candidate_snapshot = SnapshotId(r.digest()?);
    let patch = PatchId(r.digest()?);
    let analysis = r.digest()?;
    let intent = r.digest()?;
    let mut evidence = BTreeMap::new();
    for _ in 0..r.count()? {
        let check = CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        if evidence.insert(check, EvidenceId(r.digest()?)).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    r.end()?;
    let request = CommitRequest {
        transaction,
        attempt,
        authorization_grant,
        fence,
        base_snapshot,
        candidate_snapshot,
        patch,
        analysis,
        intent,
        evidence,
        key,
    };
    canonical(
        request.digest(scope, principal),
        expected,
        &request.canonical_bytes(scope, principal),
        bytes,
    )?;
    Ok(request)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeIntentRecord {
    scope: Scope,
    snapshot: SnapshotId,
    intent: TransactionIntent,
    digest: Digest,
}

impl NativeIntentRecord {
    pub const FORMAT: &str = "celetixo/native-intent-record/v1";
    pub fn new(snapshot: &Snapshot, intent: TransactionIntent) -> Result<Self, RestoreError> {
        crate::engine::validate_intent(&intent).map_err(|_| RestoreError::InvalidDomainValue)?;
        let mut record = Self {
            scope: snapshot.scope().clone(),
            snapshot: snapshot.id(),
            intent,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn intent(&self) -> &TransactionIntent {
        &self.intent
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, self.snapshot);
        c.str(&self.intent.objective);
        self.intent.accesses.encode(&mut c);
        c.u64(self.intent.invariants.len() as u64);
        for invariant in &self.intent.invariants {
            c.str(invariant.id.as_str());
            match &invariant.condition {
                Condition::Equals { element, value } => {
                    c.u64(0);
                    c.str(element.as_str());
                    c.i64(*value);
                }
                Condition::Present(element) => {
                    c.u64(1);
                    c.str(element.as_str());
                }
                Condition::SumAtMost { elements, maximum } => {
                    c.u64(2);
                    encode_elements(&mut c, elements);
                    c.i64(*maximum);
                }
            }
        }
        c.u64(self.intent.required_checks.len() as u64);
        for check in &self.intent.required_checks {
            encode_check(&mut c, check);
        }
        c.u64(self.intent.depends_on.len() as u64);
        for id in &self.intent.depends_on {
            c.str(id.as_str());
        }
        c.into_bytes()
    }
}

pub fn restore_native_intent(
    snapshot: &Snapshot,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<NativeIntentRecord, RestoreError> {
    let mut r = Reader::new(expected, NativeIntentRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, snapshot)?;
    let objective = r.text()?;
    let accesses = decode_access(&mut r)?;
    let mut invariants = Vec::new();
    for _ in 0..r.count()? {
        let id = InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        let condition = match r.number()? {
            0 => Condition::Equals {
                element: element(&mut r)?,
                value: r.signed()?,
            },
            1 => Condition::Present(element(&mut r)?),
            2 => Condition::SumAtMost {
                elements: decode_elements(&mut r)?,
                maximum: r.signed()?,
            },
            _ => return Err(RestoreError::Encoding),
        };
        invariants.push(Invariant { id, condition });
    }
    let mut required_checks = Vec::new();
    for _ in 0..r.count()? {
        required_checks.push(decode_check(&mut r)?);
    }
    let mut depends_on = BTreeSet::new();
    for _ in 0..r.count()? {
        if !depends_on.insert(TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?) {
            return Err(RestoreError::NonCanonical);
        }
    }
    r.end()?;
    let record = NativeIntentRecord::new(
        snapshot,
        TransactionIntent {
            objective,
            accesses,
            invariants,
            required_checks,
            depends_on,
        },
    )?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

pub fn restore_access_set(
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<AccessSet, RestoreError> {
    let mut r = Reader::new(expected, "celetixo/access-set/v1", bytes, limits)?;
    let access = decode_access(&mut r)?;
    r.end()?;
    canonical(access.digest(), expected, &access.canonical_bytes(), bytes)?;
    Ok(access)
}

pub fn restore_reference_patch(
    snapshot: &Snapshot,
    expected: PatchId,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<Patch, RestoreError> {
    let mut r = Reader::new(expected.0, "celetixo/patch/v1", bytes, limits)?;
    if SnapshotId(r.digest()?) != snapshot.id() {
        return Err(RestoreError::BindingMismatch);
    }
    let mut changes = BTreeMap::new();
    for _ in 0..r.count()? {
        let id = element(&mut r)?;
        let change = Change {
            expected: optional_value(&mut r)?,
            value: optional_value(&mut r)?,
        };
        if changes.insert(id, change).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    r.end()?;
    let patch = Patch::new(snapshot.id(), changes)?;
    canonical(patch.id().0, expected.0, &patch.canonical_bytes(), bytes)?;
    Ok(patch)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactAnalysisRecord {
    scope: Scope,
    snapshot: SnapshotId,
    analysis: ImpactAnalysis,
    digest: Digest,
}

impl ImpactAnalysisRecord {
    pub const FORMAT: &str = "celetixo/impact-analysis-record/v1";
    pub fn new(snapshot: &Snapshot, analysis: ImpactAnalysis) -> Result<Self, RestoreError> {
        if analysis.stamp().snapshot != snapshot.id() {
            return Err(RestoreError::BindingMismatch);
        }
        if !analysis.stamp().is_valid() {
            return Err(RestoreError::InvalidDomainValue);
        }
        let mut record = Self {
            scope: snapshot.scope().clone(),
            snapshot: snapshot.id(),
            analysis,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn analysis(&self) -> &ImpactAnalysis {
        &self.analysis
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, self.snapshot);
        let stamp = self.analysis.stamp();
        c.u64(stamp.index_generation);
        c.str(&stamp.analyzer);
        c.str(&stamp.version);
        c.digest(stamp.configuration);
        self.analysis.derived().encode(&mut c);
        encode_completeness(&mut c, self.analysis.completeness());
        c.into_bytes()
    }
}

pub fn restore_impact_record(
    snapshot: &Snapshot,
    expected_generation: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<ImpactAnalysisRecord, RestoreError> {
    let mut r = Reader::new(expected, ImpactAnalysisRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, snapshot)?;
    let index_generation = r.number()?;
    if index_generation != expected_generation {
        return Err(RestoreError::BindingMismatch);
    }
    let stamp = AnalysisStamp {
        snapshot: snapshot.id(),
        index_generation,
        analyzer: r.text()?,
        version: r.text()?,
        configuration: r.digest()?,
    };
    let derived = decode_access(&mut r)?;
    let completeness = decode_completeness(&mut r)?;
    r.end()?;
    let record =
        ImpactAnalysisRecord::new(snapshot, ImpactAnalysis::new(stamp, derived, completeness))?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

/// The envelope retains full artifact bytes. Its address is distinct from the
/// existing evidence identity, which addresses artifact digests. Neither proves
/// that an authorized verifier admitted this evidence to a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationEvidenceRecord {
    scope: Scope,
    snapshot: SnapshotId,
    evidence: VerificationEvidence,
    digest: Digest,
}

impl VerificationEvidenceRecord {
    pub const FORMAT: &str = "celetixo/verification-evidence-record/v1";
    pub fn new(snapshot: &Snapshot, evidence: VerificationEvidence) -> Result<Self, RestoreError> {
        if evidence.bindings().base_snapshot != snapshot.id() {
            return Err(RestoreError::BindingMismatch);
        }
        let mut record = Self {
            scope: snapshot.scope().clone(),
            snapshot: snapshot.id(),
            evidence,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn evidence(&self) -> &VerificationEvidence {
        &self.evidence
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, self.snapshot);
        let bindings = self.evidence.bindings();
        c.str(bindings.transaction.as_str());
        c.u64(bindings.attempt);
        c.digest(bindings.candidate_snapshot.0);
        c.digest(bindings.patch.0);
        c.digest(bindings.analysis);
        encode_check(&mut c, self.evidence.check());
        match self.evidence.result() {
            CheckResult::Passed { exit_code } => {
                c.u64(0);
                c.i64(i64::from(*exit_code));
            }
            CheckResult::Failed { exit_code } => {
                c.u64(1);
                c.i64(i64::from(*exit_code));
            }
            CheckResult::Incomplete { reason } => {
                c.u64(2);
                c.str(reason);
            }
        }
        let p = self.evidence.provenance();
        c.str(p.verifier.as_str());
        c.str(&p.runner);
        c.str(&p.run_id);
        c.u64(p.started_at);
        c.u64(p.finished_at);
        c.u64(self.evidence.diagnostics().len() as u64);
        for diagnostic in self.evidence.diagnostics() {
            c.str(diagnostic);
        }
        c.u64(self.evidence.artifacts().len() as u64);
        for artifact in self.evidence.artifacts() {
            c.str(artifact.name());
            c.str(artifact.media_type());
            c.bytes(artifact.content());
        }
        c.into_bytes()
    }
}

pub fn restore_verification_record(
    snapshot: &Snapshot,
    expected_evidence: EvidenceId,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<VerificationEvidenceRecord, RestoreError> {
    let mut r = Reader::new(expected, VerificationEvidenceRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, snapshot)?;
    let bindings = EvidenceBindings {
        transaction: TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        attempt: r.number()?,
        base_snapshot: snapshot.id(),
        candidate_snapshot: SnapshotId(r.digest()?),
        patch: PatchId(r.digest()?),
        analysis: r.digest()?,
    };
    let check = decode_check(&mut r)?;
    let result = match r.number()? {
        0 => CheckResult::Passed {
            exit_code: signed_exit(&mut r)?,
        },
        1 => CheckResult::Failed {
            exit_code: signed_exit(&mut r)?,
        },
        2 => CheckResult::Incomplete { reason: r.text()? },
        _ => return Err(RestoreError::Encoding),
    };
    let provenance = ExecutionProvenance {
        verifier: PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        runner: r.text()?,
        run_id: r.text()?,
        started_at: r.number()?,
        finished_at: r.number()?,
    };
    let mut diagnostics = Vec::new();
    for _ in 0..r.count()? {
        diagnostics.push(r.text()?);
    }
    let mut artifacts = Vec::new();
    for _ in 0..r.count()? {
        artifacts.push(Artifact::new(r.text()?, r.text()?, r.field()?.to_vec())?);
    }
    r.end()?;
    let evidence =
        VerificationEvidence::new(bindings, check, result, provenance, diagnostics, artifacts)?;
    if evidence.id() != expected_evidence {
        return Err(RestoreError::BindingMismatch);
    }
    let record = VerificationEvidenceRecord::new(snapshot, evidence)?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

fn context(scope: &Scope, snapshot: SnapshotId) -> Canonical {
    let mut c = Canonical::default();
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
    c.digest(snapshot.0);
    c
}
fn validate_context(r: &mut Reader<'_>, snapshot: &Snapshot) -> Result<(), RestoreError> {
    if r.scope()? != *snapshot.scope() || SnapshotId(r.digest()?) != snapshot.id() {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(())
}
fn canonical(
    actual: Digest,
    expected: Digest,
    encoded: &[u8],
    bytes: &[u8],
) -> Result<(), RestoreError> {
    if actual != expected || encoded != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(())
}
fn element(r: &mut Reader<'_>) -> Result<ElementId, RestoreError> {
    ElementId::new(r.text()?).map_err(|_| RestoreError::Identity)
}
fn encode_elements(c: &mut Canonical, elements: &BTreeSet<ElementId>) {
    c.u64(elements.len() as u64);
    for id in elements {
        c.str(id.as_str());
    }
}
fn decode_elements(r: &mut Reader<'_>) -> Result<BTreeSet<ElementId>, RestoreError> {
    let mut result = BTreeSet::new();
    for _ in 0..r.count()? {
        if !result.insert(element(r)?) {
            return Err(RestoreError::NonCanonical);
        }
    }
    Ok(result)
}
fn decode_access(r: &mut Reader<'_>) -> Result<AccessSet, RestoreError> {
    let reads = decode_elements(r)?;
    let writes = decode_elements(r)?;
    let invariant_scopes = decode_elements(r)?;
    let mut predicates = BTreeSet::new();
    for _ in 0..r.count()? {
        if r.text()? != "prefix/v1" {
            return Err(RestoreError::Encoding);
        }
        if !predicates.insert(Predicate::Prefix(r.text()?)) {
            return Err(RestoreError::NonCanonical);
        }
    }
    Ok(AccessSet {
        reads,
        writes,
        invariant_scopes,
        predicates,
    })
}
fn encode_check(c: &mut Canonical, check: &CheckSpec) {
    c.str(check.id().as_str());
    c.u64(check.command().len() as u64);
    for arg in check.command() {
        c.str(arg);
    }
    c.str(check.toolchain());
    c.digest(check.environment());
    c.digest(check.configuration());
}
fn decode_check(r: &mut Reader<'_>) -> Result<CheckSpec, RestoreError> {
    let id = CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let mut command = Vec::new();
    for _ in 0..r.count()? {
        command.push(r.text()?);
    }
    Ok(CheckSpec::new(
        id,
        command,
        r.text()?,
        r.digest()?,
        r.digest()?,
    )?)
}
fn optional_value(r: &mut Reader<'_>) -> Result<Option<i64>, RestoreError> {
    match r.text()?.as_str() {
        "absent" => Ok(None),
        "present" => Ok(Some(r.signed()?)),
        _ => Err(RestoreError::Encoding),
    }
}
fn signed_exit(r: &mut Reader<'_>) -> Result<i32, RestoreError> {
    i32::try_from(r.signed()?).map_err(|_| RestoreError::Encoding)
}
fn encode_completeness(c: &mut Canonical, completeness: &Completeness) {
    match completeness {
        Completeness::Complete => c.u64(0),
        Completeness::Incomplete { reasons } => {
            c.u64(1);
            c.u64(reasons.len() as u64);
            for reason in reasons {
                c.str(reason);
            }
        }
    }
}
fn decode_completeness(r: &mut Reader<'_>) -> Result<Completeness, RestoreError> {
    match r.number()? {
        0 => Ok(Completeness::Complete),
        1 => {
            let mut reasons = Vec::new();
            for _ in 0..r.count()? {
                reasons.push(r.text()?);
            }
            Ok(Completeness::Incomplete { reasons })
        }
        _ => Err(RestoreError::Encoding),
    }
}
