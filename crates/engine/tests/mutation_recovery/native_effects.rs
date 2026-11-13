//! Original effect receipts survive later evidence selection and fresh attempts.
use super::*;
use celetixo_engine::evidence::{
    Artifact, CheckResult, EvidenceBindings, ExecutionProvenance, VerificationEvidence,
};
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::transaction::TransactionState;

struct Native {
    engine: Engine,
    admin: Administrator,
    owner: Session,
    verifier: Session,
    base: Snapshot,
    backend: Arc<FaultBackend>,
}
impl Native {
    fn new(label: &str) -> Self {
        let scope = scope(label);
        let backend = Arc::new(FaultBackend {
            records: Mutex::new(NativeRecords::empty(scope.clone())),
            fault: AtomicU8::new(0),
        });
        let (engine, admin) = Engine::with_authority_backend(backend.clone());
        let owner = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                scope.clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("verifier").unwrap(),
                scope.clone(),
                [Permission::Read, Permission::Verify].into(),
            )
            .unwrap();
        let base = snapshot(scope);
        engine.register(&owner, base.clone(), model()).unwrap();
        Self {
            engine,
            admin,
            owner,
            verifier,
            base,
            backend,
        }
    }
    fn scope(&self) -> &Scope {
        self.base.scope()
    }
    fn prepare(&self, name: &str, write: &str) -> Patch {
        self.prepare_reading(name, write, &[])
    }
    fn prepare_reading(&self, name: &str, write: &str, reads: &[&str]) -> Patch {
        let element = ElementId::new(write).unwrap();
        let mut input = intent();
        input.accesses.writes = [element.clone()].into();
        input.accesses.reads = reads
            .iter()
            .map(|read| ElementId::new(*read).unwrap())
            .collect();
        self.engine
            .propose(&self.owner, self.scope(), id(name), self.base.id(), input)
            .unwrap();
        self.engine
            .analyze(&self.owner, self.scope(), &id(name))
            .unwrap();
        self.engine
            .reserve(&self.owner, self.scope(), &id(name), 120_000)
            .unwrap();
        self.engine
            .execute(&self.owner, self.scope(), &id(name))
            .unwrap();
        Patch::new(
            self.base.id(),
            [(
                element.clone(),
                Change {
                    expected: self.base.state().get(&element).copied(),
                    value: Some(10),
                },
            )]
            .into(),
        )
        .unwrap()
    }
    fn evidence(&self, name: &str, run: &str, passed: bool) -> VerificationEvidence {
        let bindings = self
            .engine
            .bindings(&self.owner, self.scope(), &id(name))
            .unwrap();
        self.evidence_for(bindings, run, passed)
    }
    fn evidence_for(
        &self,
        bindings: EvidenceBindings,
        run: &str,
        passed: bool,
    ) -> VerificationEvidence {
        VerificationEvidence::new(
            bindings,
            intent().required_checks[0].clone(),
            if passed {
                CheckResult::Passed { exit_code: 0 }
            } else {
                CheckResult::Failed { exit_code: 1 }
            },
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "reference-not-executed/v1".into(),
                run_id: run.into(),
                started_at: 0,
                finished_at: 0,
            },
            vec!["metadata only; no check command executed".into()],
            vec![Artifact::new("reference", "text/plain", b"metadata only".to_vec()).unwrap()],
        )
        .unwrap()
    }
    fn record(
        &self,
        name: &str,
        operation: &str,
        evidence: VerificationEvidence,
    ) -> MutationResponse {
        self.engine
            .mutate(
                &self.verifier,
                self.scope(),
                key(operation),
                MutationRequest::RecordEvidence {
                    transaction: id(name),
                    evidence: Box::new(evidence),
                },
            )
            .unwrap()
    }
    fn commit(&self, name: &str, operation: &str) -> Result<CommitReceipt, EngineError> {
        let request = self.engine.prepare_commit_request(
            &self.owner,
            self.scope(),
            &id(name),
            key(operation),
        )?;
        self.engine
            .commit_prepared(&self.owner, self.scope(), request)
    }
    fn reopen(&mut self) {
        let (engine, admin) = Engine::with_authority_backend(self.backend.clone());
        self.owner = admin
            .authenticate_existing_session(self.scope(), 1, |_, _| Ok(()))
            .unwrap();
        self.verifier = admin
            .authenticate_existing_session(self.scope(), 2, |_, _| Ok(()))
            .unwrap();
        self.engine = engine;
        self.admin = admin;
    }
}

#[test]
fn conflicting_revalidation_keeps_complete_original_report_after_abort_and_reopen() {
    let mut h = Native::new("native-conflict-replay");
    let reader = h.prepare_reading("reader", "y", &["x"]);
    h.engine
        .submit_patch(&h.owner, h.scope(), &id("reader"), reader)
        .unwrap();
    h.record(
        "reader",
        "reader-check",
        h.evidence("reader", "reader", true),
    );
    let writer = h.prepare("writer", "x");
    h.engine
        .submit_patch(&h.owner, h.scope(), &id("writer"), writer)
        .unwrap();
    h.record(
        "writer",
        "writer-check",
        h.evidence("writer", "writer", true),
    );
    h.commit("writer", "advance").unwrap();
    assert!(matches!(
        h.commit("reader", "stale"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let request = MutationRequest::Revalidate {
        transaction: id("reader"),
    };
    let original = h
        .engine
        .mutate(
            &h.owner,
            h.scope(),
            key("conflicting-revalidation"),
            request.clone(),
        )
        .unwrap();
    let Err(EngineError::Conflict(reports)) = &original.outcome else {
        panic!("{original:?}");
    };
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].contender, id("reader"));
    assert_eq!(reports[0].committed, id("writer"));
    assert_eq!(reports[0].committed_revision, Revision(1));
    assert!(
        reports[0]
            .contender_provenance
            .explicit
            .reads
            .contains(&ElementId::new("x").unwrap())
    );
    let records = h.backend.records.lock().unwrap().clone();
    for field in 0..3 {
        let mut corrupt = records.clone();
        let record = corrupt
            .mutations
            .values_mut()
            .find(|r| matches!(r.outcome, Err(EngineError::Conflict(_))))
            .unwrap();
        let Err(EngineError::Conflict(reports)) = &mut record.outcome else {
            unreachable!()
        };
        match field {
            0 => reports[0].conflicts[0].element = ElementId::new("unrelated").unwrap(),
            1 => {
                reports[0]
                    .contender_provenance
                    .explicit
                    .reads
                    .insert(ElementId::new("invented-read").unwrap());
            }
            _ => {
                reports[0]
                    .committed_provenance
                    .explicit
                    .writes
                    .insert(ElementId::new("invented-write").unwrap());
            }
        }
        assert!(
            AuthorityState::from_native_records(corrupt, Default::default()).is_err(),
            "accepted substituted conflict field {field}"
        );
    }
    h.engine
        .abort(
            &h.owner,
            h.scope(),
            &id("reader"),
            "rejected assumptions".into(),
        )
        .unwrap();
    h.reopen();
    let replay = h
        .engine
        .mutate(
            &h.owner,
            h.scope(),
            key("conflicting-revalidation"),
            request,
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, original.outcome);
    assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
    assert_eq!(
        h.engine.current(&h.owner, h.scope()).unwrap().revision(),
        Revision(1)
    );
}

#[test]
fn normalized_recovery_rejects_substituted_patch_bindings_and_evidence_results() {
    let h = Native::new("native-effect-tampering");
    let patch = h.prepare("a", "x");
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("patch"),
            MutationRequest::SubmitPatch {
                transaction: id("a"),
                patch,
            },
        )
        .unwrap();
    h.record("a", "evidence", h.evidence("a", "original", true));
    let original = h.backend.records.lock().unwrap().clone();
    assert!(AuthorityState::from_native_records(original.clone(), Default::default()).is_ok());
    for field in 0..6 {
        let mut changed = original.clone();
        let record = changed
            .mutations
            .values_mut()
            .find(|record| matches!(record.outcome, Ok(MutationValue::Bindings { .. })))
            .unwrap();
        let Ok(MutationValue::Bindings { bindings }) = &mut record.outcome else {
            unreachable!()
        };
        match field {
            0 => bindings.base_snapshot = SnapshotId(Digest::of("tamper/v1", b"base")),
            1 => bindings.candidate_snapshot = SnapshotId(Digest::of("tamper/v1", b"candidate")),
            2 => bindings.patch = PatchId(Digest::of("tamper/v1", b"patch")),
            3 => bindings.analysis = Digest::of("tamper/v1", b"analysis"),
            4 => bindings.attempt += 1,
            _ => bindings.transaction = id("other"),
        }
        assert!(
            AuthorityState::from_native_records(changed, Default::default()).is_err(),
            "accepted changed binding field {field}"
        );
    }
    let mut duplicate = original.clone();
    let (actor, record) = duplicate
        .mutations
        .iter()
        .find(|(_, record)| matches!(record.outcome, Ok(MutationValue::Bindings { .. })))
        .map(|(key, record)| (key.actor.clone(), record.clone()))
        .unwrap();
    duplicate.mutations.insert(
        MutationKey {
            actor,
            key: key("forged-second-patch-effect"),
        },
        record,
    );
    assert!(AuthorityState::from_native_records(duplicate, Default::default()).is_err());
    let mut changed = original;
    let record = changed
        .mutations
        .values_mut()
        .find(|record| matches!(record.outcome, Ok(MutationValue::Evidence { .. })))
        .unwrap();
    record.outcome = Ok(MutationValue::Evidence {
        evidence: EvidenceId(Digest::of("tamper/v1", b"evidence")),
    });
    assert!(AuthorityState::from_native_records(changed, Default::default()).is_err());
}

#[test]
fn original_patch_and_evidence_replies_do_not_reselect_stale_evidence_after_revalidation() {
    let mut h = Native::new("native-effect-replay");
    let patch = h.prepare("a", "x");
    let submit = MutationRequest::SubmitPatch {
        transaction: id("a"),
        patch,
    };
    let submitted = h
        .engine
        .mutate(&h.owner, h.scope(), key("patch"), submit.clone())
        .unwrap();
    assert!(
        matches!(&submitted.outcome,Ok(MutationValue::Bindings {bindings}) if bindings.attempt==0)
    );
    let failed = h.evidence("a", "first-failed", false);
    let failed_reply = h.record("a", "failed", failed.clone());
    let passed = h.evidence("a", "second-passed", true);
    let passed_reply = h.record("a", "passed", passed.clone());
    h.reopen();
    let selected = h.engine.attempts(&h.owner, h.scope(), &id("a")).unwrap()[0]
        .evidence
        .clone();
    assert_eq!(
        selected.values().copied().collect::<Vec<_>>(),
        vec![passed.id()]
    );
    let old = h.record("a", "failed", failed.clone());
    assert!(old.replayed);
    assert_eq!(old.outcome, failed_reply.outcome);
    assert_eq!(
        h.engine.attempts(&h.owner, h.scope(), &id("a")).unwrap()[0].evidence,
        selected
    );
    let other = h.prepare("b", "y");
    h.engine
        .submit_patch(&h.owner, h.scope(), &id("b"), other)
        .unwrap();
    h.record("b", "b-evidence", h.evidence("b", "b-passed", true));
    h.commit("b", "advance").unwrap();
    assert!(matches!(
        h.commit("a", "stale"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let before = h.engine.attempts(&h.owner, h.scope(), &id("a")).unwrap();
    let MutationRequest::SubmitPatch { patch, .. } = &submit else {
        unreachable!()
    };
    assert_eq!(
        h.engine
            .submit_patch(&h.owner, h.scope(), &id("a"), patch.clone()),
        Err(EngineError::IllegalTransition {
            from: TransactionState::RevalidationRequired,
            to: TransactionState::Validating
        })
    );
    let rejected = h
        .engine
        .mutate(
            &h.owner,
            h.scope(),
            key("replace-old-attempt"),
            submit.clone(),
        )
        .unwrap();
    assert_eq!(
        rejected.outcome,
        Err(EngineError::IllegalTransition {
            from: TransactionState::RevalidationRequired,
            to: TransactionState::Validating
        })
    );
    assert_eq!(
        h.engine.attempts(&h.owner, h.scope(), &id("a")).unwrap(),
        before
    );
    let request = MutationRequest::Revalidate {
        transaction: id("a"),
    };
    let revalidated = h
        .engine
        .mutate(&h.owner, h.scope(), key("revalidate"), request.clone())
        .unwrap();
    assert!(
        matches!(&revalidated.outcome,Ok(MutationValue::Bindings {bindings}) if bindings.attempt==1)
    );
    h.reopen();
    for (name, request, expected) in [
        ("patch", submit, submitted),
        ("revalidate", request, revalidated),
    ] {
        let replay = h
            .engine
            .mutate(&h.owner, h.scope(), key(name), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
        assert_eq!(replay.recorded_at_ms, expected.recorded_at_ms);
    }
    let old = h.record("a", "passed", passed.clone());
    assert!(old.replayed);
    assert_eq!(old.outcome, passed_reply.outcome);
    assert!(
        h.engine.attempts(&h.owner, h.scope(), &id("a")).unwrap()[1]
            .evidence
            .is_empty()
    );
    assert_eq!(
        h.record("a", "new-key-old-evidence", passed).outcome,
        Err(EngineError::EvidenceMismatch)
    );
    h.record(
        "a",
        "fresh-evidence",
        h.evidence("a", "fresh-attempt", true),
    );
    h.commit("a", "final").unwrap();
    h.reopen();
    assert_eq!(
        h.engine.current(&h.owner, h.scope()).unwrap().revision(),
        Revision(2)
    );
}

#[test]
fn rejected_patch_and_revalidation_requests_keep_their_original_bodies() {
    let mut h = Native::new("native-rejected-effects");
    let patch = h.prepare("work", "x");
    let wrong = Patch::new(
        SnapshotId(Digest::of("wrong-base/v1", b"original")),
        patch.changes().clone(),
    )
    .unwrap();
    let request = MutationRequest::SubmitPatch {
        transaction: id("work"),
        patch: wrong,
    };
    let rejected = h
        .engine
        .mutate(&h.owner, h.scope(), key("wrong"), request.clone())
        .unwrap();
    assert_eq!(rejected.outcome, Err(EngineError::WrongPatchBase));
    let revalidate = MutationRequest::Revalidate {
        transaction: id("work"),
    };
    let empty = h
        .engine
        .mutate(&h.owner, h.scope(), key("no-patch"), revalidate.clone())
        .unwrap();
    assert_eq!(empty.outcome, Err(EngineError::WrongPatchBase));
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("patch"),
            MutationRequest::SubmitPatch {
                transaction: id("work"),
                patch,
            },
        )
        .unwrap();
    h.reopen();
    for (name, request, expected) in [
        ("wrong", request, rejected),
        ("no-patch", revalidate, empty),
    ] {
        let actual = h
            .engine
            .mutate(&h.owner, h.scope(), key(name), request)
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
    }
    let mut bindings = h.engine.bindings(&h.owner, h.scope(), &id("work")).unwrap();
    bindings.attempt = 999;
    let evidence = h.evidence_for(bindings, "wrong-attempt", true);
    let bad = h.record("work", "bad-evidence", evidence.clone());
    assert_eq!(bad.outcome, Err(EngineError::EvidenceMismatch));
    h.reopen();
    assert_eq!(
        h.record("work", "bad-evidence", evidence).outcome,
        bad.outcome
    );
}
