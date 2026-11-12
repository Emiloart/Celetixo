//! Exact historical domain decisions. Debug/Display strings are never an
//! authority encoding, and current state cannot recreate a previous rejection.
use super::lifecycle::{
    canonical, decode_access, decode_completeness, decode_elements, element, encode_completeness,
    encode_elements, optional_value,
};
use super::*;
use crate::access::{AnalysisStamp, Conflict, ConflictKind, InvariantViolation, Predicate};
use crate::engine::{AccessProvenance, CommitReceipt, ConflictAction, ConflictReport, EngineError};
use crate::ids::{Canonical, CheckId, InvariantId, PatchId, TransactionId};
use crate::patch::PatchError;
use crate::transaction::TransactionState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessProvenanceRecord {
    scope: Scope,
    transaction: TransactionId,
    attempt: u64,
    provenance: AccessProvenance,
    digest: Digest,
}
impl AccessProvenanceRecord {
    pub const FORMAT: &str = "celetixo/access-provenance-record/v1";
    pub fn new(
        snapshot: &Snapshot,
        transaction: TransactionId,
        attempt: u64,
        provenance: AccessProvenance,
    ) -> Result<Self, RestoreError> {
        if provenance.analysis.snapshot != snapshot.id() {
            return Err(RestoreError::BindingMismatch);
        }
        validate_provenance(&provenance)?;
        let mut record = Self {
            scope: snapshot.scope().clone(),
            transaction,
            attempt,
            provenance,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn provenance(&self) -> &AccessProvenance {
        &self.provenance
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn transaction(&self) -> &TransactionId {
        &self.transaction
    }
    pub fn attempt(&self) -> u64 {
        self.attempt
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, &self.transaction, self.attempt);
        encode_provenance(&mut c, &self.provenance);
        c.into_bytes()
    }
}
pub fn restore_access_provenance(
    snapshot: &Snapshot,
    transaction: &TransactionId,
    attempt: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<AccessProvenanceRecord, RestoreError> {
    let mut r = Reader::new(expected, AccessProvenanceRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, snapshot.scope(), transaction, attempt)?;
    let provenance = decode_provenance(&mut r)?;
    r.end()?;
    let record = AccessProvenanceRecord::new(snapshot, transaction.clone(), attempt, provenance)?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcomeRecord {
    scope: Scope,
    transaction: TransactionId,
    attempt: u64,
    outcome: Result<CommitReceipt, EngineError>,
    digest: Digest,
}
impl CommitOutcomeRecord {
    pub const FORMAT: &str = "celetixo/commit-outcome-record/v1";
    pub fn new(
        scope: Scope,
        transaction: TransactionId,
        attempt: u64,
        outcome: Result<CommitReceipt, EngineError>,
    ) -> Result<Self, RestoreError> {
        match &outcome {
            Ok(receipt)
                if receipt.transaction != transaction
                    || receipt.attempt != attempt
                    || receipt.revision.0 == 0 =>
            {
                return Err(RestoreError::BindingMismatch);
            }
            Err(EngineError::Conflict(reports)) => {
                if reports.is_empty() {
                    return Err(RestoreError::InvalidDomainValue);
                }
                for report in reports {
                    if report.contender != transaction
                        || report.contender_attempt != attempt
                        || report.committed_revision.0 == 0
                        || report.conflicts.is_empty()
                    {
                        return Err(RestoreError::BindingMismatch);
                    }
                    validate_provenance(&report.contender_provenance)?;
                    validate_provenance(&report.committed_provenance)?;
                    // Full dependency validation also needs each transaction's
                    // invariant definitions; provenance alone cannot recover
                    // the distinction between declared scopes and invariant
                    // reads. The scoped state loader must check those facts.
                    if report.conflicts.windows(2).any(|pair| pair[0] >= pair[1]) {
                        return Err(RestoreError::InvalidDomainValue);
                    }
                    for conflict in &report.conflicts {
                        match (conflict.kind, &conflict.predicate) {
                            (
                                ConflictKind::PredicateWrite | ConflictKind::WritePredicate,
                                Some(predicate),
                            ) if predicate.matches(&conflict.element) => {}
                            (ConflictKind::PredicateWrite | ConflictKind::WritePredicate, _) => {
                                return Err(RestoreError::InvalidDomainValue);
                            }
                            (_, None) => {}
                            (_, Some(_)) => return Err(RestoreError::InvalidDomainValue),
                        }
                    }
                }
            }
            _ => {}
        }
        let mut record = Self {
            scope,
            transaction,
            attempt,
            outcome,
            digest: Digest::from_bytes([0; 32]),
        };
        let bytes = record.encode()?;
        record.digest = Digest::of(Self::FORMAT, &bytes);
        Ok(record)
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn transaction(&self) -> &TransactionId {
        &self.transaction
    }
    pub fn attempt(&self) -> u64 {
        self.attempt
    }
    pub fn outcome(&self) -> &Result<CommitReceipt, EngineError> {
        &self.outcome
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.encode()
            .expect("constructed outcome has a supported encoding")
    }
    fn encode(&self) -> Result<Vec<u8>, RestoreError> {
        let mut c = context(&self.scope, &self.transaction, self.attempt);
        match &self.outcome {
            Ok(receipt) => {
                c.str("Accepted");
                c.u64(receipt.revision.0);
                c.digest(receipt.snapshot.0);
                c.digest(receipt.patch.0);
            }
            Err(error) => {
                c.str("Rejected");
                encode_error(&mut c, error)?;
            }
        }
        Ok(c.into_bytes())
    }
}
pub fn restore_commit_outcome(
    scope: &Scope,
    transaction: &TransactionId,
    attempt: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<CommitOutcomeRecord, RestoreError> {
    let mut r = Reader::new(expected, CommitOutcomeRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, scope, transaction, attempt)?;
    let outcome = match r.text()?.as_str() {
        "Accepted" => Ok(CommitReceipt {
            transaction: transaction.clone(),
            attempt,
            revision: Revision(r.number()?),
            snapshot: SnapshotId(r.digest()?),
            patch: PatchId(r.digest()?),
        }),
        "Rejected" => Err(decode_error(&mut r)?),
        _ => return Err(RestoreError::Encoding),
    };
    r.end()?;
    let record = CommitOutcomeRecord::new(scope.clone(), transaction.clone(), attempt, outcome)?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}
pub(super) fn context(scope: &Scope, transaction: &TransactionId, attempt: u64) -> Canonical {
    let mut c = Canonical::default();
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
    c.str(transaction.as_str());
    c.u64(attempt);
    c
}
pub(super) fn validate_context(
    r: &mut Reader<'_>,
    scope: &Scope,
    transaction: &TransactionId,
    attempt: u64,
) -> Result<(), RestoreError> {
    if r.scope()? != *scope || r.text()? != transaction.as_str() || r.number()? != attempt {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(())
}
fn validate_provenance(p: &AccessProvenance) -> Result<(), RestoreError> {
    if !p.analysis.is_valid() || p.analysis.index_generation == 0 {
        return Err(RestoreError::InvalidDomainValue);
    }
    Ok(())
}
fn encode_provenance(c: &mut Canonical, p: &AccessProvenance) {
    p.explicit.encode(c);
    p.derived.encode(c);
    encode_elements(c, &p.invariant_scopes);
    c.digest(p.analysis.snapshot.0);
    c.u64(p.analysis.index_generation);
    c.str(&p.analysis.analyzer);
    c.str(&p.analysis.version);
    c.digest(p.analysis.configuration);
    encode_completeness(c, &p.completeness);
}
fn decode_provenance(r: &mut Reader<'_>) -> Result<AccessProvenance, RestoreError> {
    let p = AccessProvenance {
        explicit: decode_access(r)?,
        derived: decode_access(r)?,
        invariant_scopes: decode_elements(r)?,
        analysis: AnalysisStamp {
            snapshot: SnapshotId(r.digest()?),
            index_generation: r.number()?,
            analyzer: r.text()?,
            version: r.text()?,
            configuration: r.digest()?,
        },
        completeness: decode_completeness(r)?,
    };
    validate_provenance(&p)?;
    Ok(p)
}
fn encode_state(c: &mut Canonical, state: TransactionState) {
    c.str(match state {
        TransactionState::Proposed => "Proposed",
        TransactionState::Analyzed => "Analyzed",
        TransactionState::Reserved => "Reserved",
        TransactionState::Executing => "Executing",
        TransactionState::Validating => "Validating",
        TransactionState::RevalidationRequired => "RevalidationRequired",
        TransactionState::Committed => "Committed",
        TransactionState::Aborted => "Aborted",
    });
}
fn decode_state(r: &mut Reader<'_>) -> Result<TransactionState, RestoreError> {
    Ok(match r.text()?.as_str() {
        "Proposed" => TransactionState::Proposed,
        "Analyzed" => TransactionState::Analyzed,
        "Reserved" => TransactionState::Reserved,
        "Executing" => TransactionState::Executing,
        "Validating" => TransactionState::Validating,
        "RevalidationRequired" => TransactionState::RevalidationRequired,
        "Committed" => TransactionState::Committed,
        "Aborted" => TransactionState::Aborted,
        _ => return Err(RestoreError::Encoding),
    })
}
fn encode_optional(c: &mut Canonical, value: Option<i64>) {
    match value {
        Some(value) => {
            c.str("present");
            c.i64(value)
        }
        None => c.str("absent"),
    }
}
fn invariant(r: &mut Reader<'_>) -> Result<InvariantId, RestoreError> {
    InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)
}
fn check(r: &mut Reader<'_>) -> Result<CheckId, RestoreError> {
    CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)
}
fn transaction(r: &mut Reader<'_>) -> Result<TransactionId, RestoreError> {
    TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)
}
fn encode_error(c: &mut Canonical, error: &EngineError) -> Result<(), RestoreError> {
    use EngineError::*;
    match error {
        Unauthorized => c.str("Unauthorized"),
        NotFound => c.str("NotFound"),
        AlreadyExists => c.str("AlreadyExists"),
        IncompleteAnalysis => c.str("IncompleteAnalysis"),
        ModelChanged => c.str("ModelChanged"),
        UnsatisfiedDependency => c.str("UnsatisfiedDependency"),
        ReservationConflict => c.str("ReservationConflict"),
        InvalidLease => c.str("InvalidLease"),
        InvalidTime => c.str("InvalidTime"),
        CounterExhausted => c.str("CounterExhausted"),
        CommitRequestRequired => c.str("CommitRequestRequired"),
        CommitRequestMismatch => c.str("CommitRequestMismatch"),
        WrongPatchBase => c.str("WrongPatchBase"),
        UndeclaredWrite => c.str("UndeclaredWrite"),
        EvidenceMismatch => c.str("EvidenceMismatch"),
        IdempotencyConflict => c.str("IdempotencyConflict"),
        SourceAnalysisRequired => c.str("SourceAnalysisRequired"),
        SourceMutationUnsupported => c.str("SourceMutationUnsupported"),
        InvalidIntent(reason) => {
            c.str("InvalidIntent");
            c.str(reason)
        }
        IllegalTransition { from, to } => {
            c.str("IllegalTransition");
            encode_state(c, *from);
            encode_state(c, *to)
        }
        RevalidationRequired { current } => {
            c.str("RevalidationRequired");
            c.digest(current.0)
        }
        MissingEvidence(check) => {
            c.str("MissingEvidence");
            c.str(check.as_str())
        }
        EvidenceNotPassed(check) => {
            c.str("EvidenceNotPassed");
            c.str(check.as_str())
        }
        Invariant(violation) => {
            c.str("Invariant");
            match violation {
                InvariantViolation::Missing { invariant, element } => {
                    c.str("Missing");
                    c.str(invariant.as_str());
                    c.str(element.as_str())
                }
                InvariantViolation::Unsatisfied { invariant } => {
                    c.str("Unsatisfied");
                    c.str(invariant.as_str())
                }
                InvariantViolation::Overflow { invariant } => {
                    c.str("Overflow");
                    c.str(invariant.as_str())
                }
            }
        }
        Patch(error) => {
            c.str("Patch");
            match error {
                PatchError::Empty => c.str("Empty"),
                PatchError::NoOp(element) => {
                    c.str("NoOp");
                    c.str(element.as_str())
                }
                PatchError::BeforeImage {
                    element,
                    expected,
                    actual,
                } => {
                    c.str("BeforeImage");
                    c.str(element.as_str());
                    encode_optional(c, *expected);
                    encode_optional(c, *actual)
                }
            }
        }
        Conflict(reports) => {
            c.str("Conflict");
            c.u64(reports.len() as u64);
            for report in reports {
                encode_report(c, report)
            }
        }
        // Infrastructure/response-loss/injection failures are not durable domain
        // outcomes. Source families need their full checked codecs before this
        // reconstruction profile can retain them; never fall back to strings.
        Authority(_)
        | StoreUnavailable
        | Snapshot(_)
        | SourceEvidence(_)
        | InjectedFailure(_)
        | ResponseLost
        | RepositoryAnalysis(_)
        | SourceAttempt(_) => return Err(RestoreError::UnsupportedDomainValue),
    };
    Ok(())
}
fn decode_error(r: &mut Reader<'_>) -> Result<EngineError, RestoreError> {
    use EngineError::*;
    Ok(match r.text()?.as_str() {
        "Unauthorized" => Unauthorized,
        "NotFound" => NotFound,
        "AlreadyExists" => AlreadyExists,
        "IncompleteAnalysis" => IncompleteAnalysis,
        "ModelChanged" => ModelChanged,
        "UnsatisfiedDependency" => UnsatisfiedDependency,
        "ReservationConflict" => ReservationConflict,
        "InvalidLease" => InvalidLease,
        "InvalidTime" => InvalidTime,
        "CounterExhausted" => CounterExhausted,
        "CommitRequestRequired" => CommitRequestRequired,
        "CommitRequestMismatch" => CommitRequestMismatch,
        "WrongPatchBase" => WrongPatchBase,
        "UndeclaredWrite" => UndeclaredWrite,
        "EvidenceMismatch" => EvidenceMismatch,
        "IdempotencyConflict" => IdempotencyConflict,
        "SourceAnalysisRequired" => SourceAnalysisRequired,
        "SourceMutationUnsupported" => SourceMutationUnsupported,
        "InvalidIntent" => InvalidIntent(r.text()?),
        "IllegalTransition" => IllegalTransition {
            from: decode_state(r)?,
            to: decode_state(r)?,
        },
        "RevalidationRequired" => RevalidationRequired {
            current: SnapshotId(r.digest()?),
        },
        "MissingEvidence" => MissingEvidence(check(r)?),
        "EvidenceNotPassed" => EvidenceNotPassed(check(r)?),
        "Invariant" => Invariant(match r.text()?.as_str() {
            "Missing" => InvariantViolation::Missing {
                invariant: invariant(r)?,
                element: element(r)?,
            },
            "Unsatisfied" => InvariantViolation::Unsatisfied {
                invariant: invariant(r)?,
            },
            "Overflow" => InvariantViolation::Overflow {
                invariant: invariant(r)?,
            },
            _ => return Err(RestoreError::Encoding),
        }),
        "Patch" => Patch(match r.text()?.as_str() {
            "Empty" => PatchError::Empty,
            "NoOp" => PatchError::NoOp(element(r)?),
            "BeforeImage" => PatchError::BeforeImage {
                element: element(r)?,
                expected: optional_value(r)?,
                actual: optional_value(r)?,
            },
            _ => return Err(RestoreError::Encoding),
        }),
        "Conflict" => {
            let mut reports = Vec::new();
            for _ in 0..r.count()? {
                reports.push(decode_report(r)?)
            }
            Conflict(reports)
        }
        _ => return Err(RestoreError::Encoding),
    })
}
fn encode_report(c: &mut Canonical, report: &ConflictReport) {
    c.str(report.contender.as_str());
    c.u64(report.contender_attempt);
    c.str(report.committed.as_str());
    c.u64(report.committed_attempt);
    c.u64(report.committed_revision.0);
    c.u64(report.conflicts.len() as u64);
    for conflict in &report.conflicts {
        c.str(match conflict.kind {
            ConflictKind::WriteWrite => "WriteWrite",
            ConflictKind::ReadWrite => "ReadWrite",
            ConflictKind::WriteRead => "WriteRead",
            ConflictKind::PredicateWrite => "PredicateWrite",
            ConflictKind::WritePredicate => "WritePredicate",
            ConflictKind::InvariantOverlap => "InvariantOverlap",
            ConflictKind::InvariantWrite => "InvariantWrite",
            ConflictKind::WriteInvariant => "WriteInvariant",
        });
        c.str(conflict.element.as_str());
        match &conflict.predicate {
            None => c.str("None"),
            Some(Predicate::Prefix(prefix)) => {
                c.str("Prefix/v1");
                c.str(prefix)
            }
        }
    }
    encode_provenance(c, &report.contender_provenance);
    encode_provenance(c, &report.committed_provenance);
    match report.action {
        ConflictAction::AbortOrNewIntent => c.str("AbortOrNewIntent"),
    }
}
fn decode_report(r: &mut Reader<'_>) -> Result<ConflictReport, RestoreError> {
    let contender = transaction(r)?;
    let contender_attempt = r.number()?;
    let committed = transaction(r)?;
    let committed_attempt = r.number()?;
    let committed_revision = Revision(r.number()?);
    let mut conflicts = Vec::new();
    for _ in 0..r.count()? {
        let kind = match r.text()?.as_str() {
            "WriteWrite" => ConflictKind::WriteWrite,
            "ReadWrite" => ConflictKind::ReadWrite,
            "WriteRead" => ConflictKind::WriteRead,
            "PredicateWrite" => ConflictKind::PredicateWrite,
            "WritePredicate" => ConflictKind::WritePredicate,
            "InvariantOverlap" => ConflictKind::InvariantOverlap,
            "InvariantWrite" => ConflictKind::InvariantWrite,
            "WriteInvariant" => ConflictKind::WriteInvariant,
            _ => return Err(RestoreError::Encoding),
        };
        let element = element(r)?;
        let predicate = match r.text()?.as_str() {
            "None" => None,
            "Prefix/v1" => Some(Predicate::Prefix(r.text()?)),
            _ => return Err(RestoreError::Encoding),
        };
        conflicts.push(Conflict {
            kind,
            element,
            predicate,
        });
    }
    let contender_provenance = decode_provenance(r)?;
    let committed_provenance = decode_provenance(r)?;
    let action = match r.text()?.as_str() {
        "AbortOrNewIntent" => ConflictAction::AbortOrNewIntent,
        _ => return Err(RestoreError::Encoding),
    };
    Ok(ConflictReport {
        contender,
        contender_attempt,
        committed,
        committed_attempt,
        committed_revision,
        conflicts,
        contender_provenance,
        committed_provenance,
        action,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::{AccessSet, Completeness};
    use crate::engine::{CommitFault, SourceAttemptError};
    use crate::snapshot::{InclusionPolicy, SnapshotManifest};
    fn scope() -> Scope {
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        }
    }
    fn tx() -> TransactionId {
        TransactionId::new("tx").unwrap()
    }
    fn e(s: &str) -> ElementId {
        ElementId::new(s).unwrap()
    }
    fn invariant_id() -> InvariantId {
        InvariantId::new("invariant").unwrap()
    }
    fn check_id() -> CheckId {
        CheckId::new("check").unwrap()
    }
    fn d(byte: u8) -> Digest {
        Digest::from_bytes([byte; 32])
    }
    fn base() -> Snapshot {
        Snapshot::new(
            scope(),
            Revision(0),
            SnapshotManifest {
                base_commit: "fixture".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "lifecycle-codec-test/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            BTreeMap::from([(e("a"), 1), (e("b"), 2)]),
        )
        .unwrap()
    }
    fn provenance() -> AccessProvenance {
        AccessProvenance {
            explicit: AccessSet {
                reads: BTreeSet::from([e("read")]),
                writes: BTreeSet::from([e("write")]),
                invariant_scopes: BTreeSet::from([e("declared")]),
                predicates: BTreeSet::from([Predicate::Prefix("missing:".into())]),
            },
            derived: AccessSet {
                reads: BTreeSet::from([e("derived")]),
                ..Default::default()
            },
            invariant_scopes: BTreeSet::from([e("declared"), e("implicit")]),
            analysis: AnalysisStamp {
                snapshot: base().id(),
                index_generation: u64::MAX,
                analyzer: "reference".into(),
                version: "v1".into(),
                configuration: d(3),
            },
            completeness: Completeness::Incomplete {
                reasons: vec!["unresolved".into()],
            },
        }
    }
    fn unhex(value: &str) -> Vec<u8> {
        value
            .trim()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    fn accepted() -> CommitOutcomeRecord {
        CommitOutcomeRecord::new(
            scope(),
            tx(),
            u64::MAX,
            Ok(CommitReceipt {
                transaction: tx(),
                attempt: u64::MAX,
                revision: Revision(u64::MAX),
                snapshot: SnapshotId(d(1)),
                patch: PatchId(d(2)),
            }),
        )
        .unwrap()
    }
    fn before_image() -> EngineError {
        EngineError::Patch(PatchError::BeforeImage {
            element: e("write"),
            expected: Some(i64::MIN),
            actual: None,
        })
    }
    fn encode_error_outcome(error: EngineError) -> CommitOutcomeRecord {
        CommitOutcomeRecord::new(scope(), tx(), u64::MAX, Err(error)).unwrap()
    }

    #[test]
    fn independent_frozen_outcome_and_provenance_layouts_preserve_exact_values() {
        let accepted = accepted();
        let rejected = encode_error_outcome(before_image());
        let provenance =
            AccessProvenanceRecord::new(&base(), tx(), u64::MAX, provenance()).unwrap();
        for (record, bytes) in [
            (
                accepted,
                unhex(include_str!(
                    "../../tests/fixtures/commit-outcome-accepted-v1.hex"
                )),
            ),
            (
                rejected,
                unhex(include_str!(
                    "../../tests/fixtures/commit-outcome-rejected-v1.hex"
                )),
            ),
        ] {
            assert_eq!(record.canonical_bytes(), bytes);
            assert_eq!(
                restore_commit_outcome(
                    &scope(),
                    &tx(),
                    u64::MAX,
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits::default()
                )
                .unwrap(),
                record
            );
        }
        let bytes = unhex(include_str!(
            "../../tests/fixtures/access-provenance-record-v1.hex"
        ));
        assert_eq!(provenance.canonical_bytes(), bytes);
        assert_eq!(
            restore_access_provenance(
                &base(),
                &tx(),
                u64::MAX,
                provenance.digest(),
                &bytes,
                DomainDecodeLimits::default()
            )
            .unwrap(),
            provenance
        );
    }
    #[test]
    fn all_supported_native_rejections_roundtrip_without_display_or_debug_interpretation() {
        use EngineError::*;
        let mut errors = vec![
            Unauthorized,
            NotFound,
            AlreadyExists,
            IncompleteAnalysis,
            ModelChanged,
            UnsatisfiedDependency,
            ReservationConflict,
            InvalidLease,
            InvalidTime,
            CounterExhausted,
            CommitRequestMismatch,
            CommitRequestRequired,
            WrongPatchBase,
            UndeclaredWrite,
            EvidenceMismatch,
            IdempotencyConflict,
            SourceAnalysisRequired,
            SourceMutationUnsupported,
            InvalidIntent("message with binary UTF-8 control \0 and unicode λ".into()),
            RevalidationRequired {
                current: SnapshotId(d(5)),
            },
            MissingEvidence(check_id()),
            EvidenceNotPassed(check_id()),
            Invariant(InvariantViolation::Missing {
                invariant: invariant_id(),
                element: e("write"),
            }),
            Invariant(InvariantViolation::Unsatisfied {
                invariant: invariant_id(),
            }),
            Invariant(InvariantViolation::Overflow {
                invariant: invariant_id(),
            }),
            Patch(PatchError::Empty),
            Patch(PatchError::NoOp(e("write"))),
            before_image(),
            Patch(PatchError::BeforeImage {
                element: e("write"),
                expected: None,
                actual: Some(i64::MAX),
            }),
        ];
        let states = [
            TransactionState::Proposed,
            TransactionState::Analyzed,
            TransactionState::Reserved,
            TransactionState::Executing,
            TransactionState::Validating,
            TransactionState::RevalidationRequired,
            TransactionState::Committed,
            TransactionState::Aborted,
        ];
        for from in states {
            for to in states {
                errors.push(IllegalTransition { from, to });
            }
        }
        for error in errors {
            let record = encode_error_outcome(error.clone());
            let restored = restore_commit_outcome(
                &scope(),
                &tx(),
                u64::MAX,
                record.digest(),
                &record.canonical_bytes(),
                DomainDecodeLimits::default(),
            )
            .unwrap();
            assert_eq!(restored.outcome(), &Err(error));
        }
        for error in [
            StoreUnavailable,
            ResponseLost,
            InjectedFailure(CommitFault::BeforePublication),
            Snapshot(crate::snapshot::SnapshotError::InvalidSuccessorRevision),
            SourceAttempt(SourceAttemptError::InvalidState),
        ] {
            assert_eq!(
                CommitOutcomeRecord::new(scope(), tx(), 0, Err(error)),
                Err(RestoreError::UnsupportedDomainValue)
            );
        }
    }
    #[test]
    fn rehashed_truncation_unknown_tags_context_substitution_and_resource_amplification_reject() {
        let record = accepted();
        let bytes = record.canonical_bytes();
        let decode = |bytes: &[u8], limits| {
            restore_commit_outcome(
                &scope(),
                &tx(),
                u64::MAX,
                Digest::of(CommitOutcomeRecord::FORMAT, bytes),
                bytes,
                limits,
            )
        };
        for end in 0..bytes.len() {
            assert!(decode(&bytes[..end], DomainDecodeLimits::default()).is_err());
        }
        let mut bad = bytes.clone();
        let offset = bad
            .windows(8)
            .position(|window| window == b"Accepted")
            .unwrap();
        bad[offset..offset + 8].copy_from_slice(b"Future!!");
        assert_eq!(
            decode(&bad, DomainDecodeLimits::default()),
            Err(RestoreError::Encoding)
        );
        let mut bad = bytes.clone();
        bad[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            decode(&bad, DomainDecodeLimits::default()),
            Err(RestoreError::Limit)
        );
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(
            decode(&bad, DomainDecodeLimits::default()),
            Err(RestoreError::NonCanonical)
        );
        for limits in [
            DomainDecodeLimits {
                encoded_bytes: 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                field_bytes: 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                collection_entries: 0,
                ..Default::default()
            },
        ] {
            assert_eq!(decode(&bytes, limits), Err(RestoreError::Limit));
        }
        assert_eq!(
            restore_commit_outcome(
                &scope(),
                &TransactionId::new("wrong").unwrap(),
                u64::MAX,
                record.digest(),
                &bytes,
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
        assert_eq!(
            restore_commit_outcome(
                &scope(),
                &tx(),
                0,
                record.digest(),
                &bytes,
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
        let mut receipt = accepted().outcome().clone().unwrap();
        receipt.attempt = 0;
        assert_eq!(
            CommitOutcomeRecord::new(scope(), tx(), u64::MAX, Ok(receipt)),
            Err(RestoreError::BindingMismatch)
        );
    }
    #[test]
    fn conflict_records_preserve_typed_provenance_and_reject_invalid_predicates_and_order() {
        let mut left = provenance();
        let mut right = provenance();
        left.completeness = Completeness::Complete;
        right.completeness = Completeness::Complete;
        left.explicit
            .predicates
            .insert(Predicate::Prefix("write".into()));
        let conflicts = left.explicit.conflicts(&right.explicit);
        assert!(!conflicts.is_empty());
        let report = ConflictReport {
            contender: tx(),
            contender_attempt: u64::MAX,
            committed: TransactionId::new("committed").unwrap(),
            committed_attempt: 0,
            committed_revision: Revision(1),
            conflicts,
            contender_provenance: left,
            committed_provenance: right,
            action: ConflictAction::AbortOrNewIntent,
        };
        let record = encode_error_outcome(EngineError::Conflict(vec![report.clone()]));
        assert_eq!(
            restore_commit_outcome(
                &scope(),
                &tx(),
                u64::MAX,
                record.digest(),
                &record.canonical_bytes(),
                DomainDecodeLimits::default()
            )
            .unwrap(),
            record
        );
        let mut bad = report.clone();
        bad.conflicts.reverse();
        assert!(
            CommitOutcomeRecord::new(
                scope(),
                tx(),
                u64::MAX,
                Err(EngineError::Conflict(vec![bad]))
            )
            .is_err()
        );
        let mut bad = report;
        let conflict = bad
            .conflicts
            .iter_mut()
            .find(|c| matches!(c.kind, ConflictKind::PredicateWrite))
            .unwrap();
        conflict.predicate = Some(Predicate::Prefix("does-not-match".into()));
        assert_eq!(
            CommitOutcomeRecord::new(
                scope(),
                tx(),
                u64::MAX,
                Err(EngineError::Conflict(vec![bad]))
            ),
            Err(RestoreError::InvalidDomainValue)
        );
    }
}
