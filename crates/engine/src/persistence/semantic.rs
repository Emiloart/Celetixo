//! Snapshot-bound immutable semantic records. Decoding restores facts, not trust.
use super::*;
use crate::access::Completeness;
use crate::ids::{Canonical, CheckId};
use crate::model::{EngineeringModel, InvalidEngineeringModel};
use crate::repository::{AnalysisGap, RepositoryAnalysis, RepositoryAnalysisInput};

/// A frozen model payload bound to an exact snapshot and scope. This is neither
/// an installed model head nor a serialized Engine/store/capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineeringModelRecord {
    scope: Scope,
    snapshot: SnapshotId,
    model: EngineeringModel,
    digest: Digest,
}

impl EngineeringModelRecord {
    pub const FORMAT: &str = "celetixo/engineering-model-record/v1";

    pub fn new(
        snapshot: &Snapshot,
        model: EngineeringModel,
    ) -> Result<Self, InvalidEngineeringModel> {
        model.validate()?;
        let mut record = Self {
            scope: snapshot.scope().clone(),
            snapshot: snapshot.id(),
            model,
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
    pub fn model(&self) -> &EngineeringModel {
        &self.model
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(self.scope.organization.as_str());
        c.str(self.scope.workspace.as_str());
        c.str(self.scope.repository.as_str());
        c.digest(self.snapshot.0);
        encode_model_fields(&mut c, &self.model);
        c.into_bytes()
    }
}

pub(crate) fn encode_model_fields(c: &mut Canonical, model: &EngineeringModel) {
    c.u64(model.generation);
    c.str(&model.analyzer);
    c.str(&model.version);
    c.digest(model.configuration);
    encode_dependencies(c, &model.dependencies);
    match &model.completeness {
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

pub fn restore_model_record(
    snapshot: &Snapshot,
    expected_generation: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<EngineeringModelRecord, RestoreError> {
    let mut r = Reader::new(expected, EngineeringModelRecord::FORMAT, bytes, limits)?;
    validate_snapshot_binding(&mut r, snapshot)?;
    let generation = r.number()?;
    if generation != expected_generation {
        return Err(RestoreError::BindingMismatch);
    }
    let model = decode_model_after_generation(&mut r, generation)?;
    r.end()?;
    let record = EngineeringModelRecord::new(snapshot, model)?;
    if record.digest() != expected || record.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(record)
}

/// Structural fields may describe an invalid original request. The admitted
/// model-record constructor above still enforces all model invariants.
pub(super) fn decode_model_fields(r: &mut Reader<'_>) -> Result<EngineeringModel, RestoreError> {
    let generation = r.number()?;
    decode_model_after_generation(r, generation)
}
fn decode_model_after_generation(
    r: &mut Reader<'_>,
    generation: u64,
) -> Result<EngineeringModel, RestoreError> {
    let analyzer = r.text()?;
    let version = r.text()?;
    let configuration = r.digest()?;
    let dependencies = decode_dependencies(r)?;
    let completeness = match r.number()? {
        0 => Completeness::Complete,
        1 => {
            let mut reasons = Vec::new();
            for _ in 0..r.count()? {
                reasons.push(r.text()?);
            }
            Completeness::Incomplete { reasons }
        }
        _ => return Err(RestoreError::Encoding),
    };
    Ok(EngineeringModel {
        generation,
        analyzer,
        version,
        configuration,
        dependencies,
        completeness,
    })
}

/// Decode the unchanged repository-analysis v1 digest preimage. The caller must
/// provide its expected generation; persisted bytes cannot choose freshness.
pub fn restore_repository_analysis(
    snapshot: &Snapshot,
    expected_index: Digest,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<RepositoryAnalysis, RestoreError> {
    let mut r = Reader::new(expected, "celetixo/repository-analysis/v1", bytes, limits)?;
    validate_snapshot_binding(&mut r, snapshot)?;
    let index_generation = r.digest()?;
    if index_generation != expected_index {
        return Err(RestoreError::BindingMismatch);
    }
    let analysis = decode_analysis_fields(
        &mut r,
        snapshot.scope().clone(),
        snapshot.id(),
        index_generation,
    )?;
    r.end()?;
    analysis
        .validate(snapshot, expected_index)
        .map_err(|_| RestoreError::BindingMismatch)?;
    if analysis.digest() != expected || analysis.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(analysis)
}
pub(super) fn decode_original_analysis(
    r: &mut Reader<'_>,
) -> Result<RepositoryAnalysis, RestoreError> {
    let scope = r.scope()?;
    let snapshot = SnapshotId(r.digest()?);
    let index_generation = r.digest()?;
    decode_analysis_fields(r, scope, snapshot, index_generation)
}
fn decode_analysis_fields(
    r: &mut Reader<'_>,
    scope: Scope,
    snapshot: SnapshotId,
    index_generation: Digest,
) -> Result<RepositoryAnalysis, RestoreError> {
    let analyzer_version = r.text()?;
    let configuration = r.digest()?;
    let elements = decode_elements(r)?;
    let dependencies = decode_dependencies(r)?;
    let mut checks = BTreeMap::new();
    for _ in 0..r.count()? {
        let element = element(r)?;
        let mut required = BTreeSet::new();
        for _ in 0..r.count()? {
            let check = CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
            if !required.insert(check) {
                return Err(RestoreError::NonCanonical);
            }
        }
        if checks.insert(element, required).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut gaps = BTreeSet::new();
    for _ in 0..r.count()? {
        let stage = r.text()?;
        let path = match r.number()? {
            0 => None,
            1 => Some(r.text()?),
            _ => return Err(RestoreError::Encoding),
        };
        let detail = r.text()?;
        if !gaps.insert(AnalysisGap {
            stage,
            path,
            detail,
        }) {
            return Err(RestoreError::NonCanonical);
        }
    }
    let analysis = RepositoryAnalysis::from_original_binding(
        scope,
        snapshot,
        RepositoryAnalysisInput {
            index_generation,
            analyzer_version,
            configuration,
            elements,
            dependencies,
            checks,
            gaps,
        },
    )?;
    Ok(analysis)
}

fn validate_snapshot_binding(r: &mut Reader<'_>, snapshot: &Snapshot) -> Result<(), RestoreError> {
    let scope = r.scope()?;
    let id = SnapshotId(r.digest()?);
    if scope != *snapshot.scope() || id != snapshot.id() {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(())
}

fn element(r: &mut Reader<'_>) -> Result<ElementId, RestoreError> {
    ElementId::new(r.text()?).map_err(|_| RestoreError::Identity)
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

fn decode_dependencies(
    r: &mut Reader<'_>,
) -> Result<BTreeMap<ElementId, BTreeSet<ElementId>>, RestoreError> {
    let mut result = BTreeMap::new();
    for _ in 0..r.count()? {
        let consumer = element(r)?;
        if result.insert(consumer, decode_elements(r)?).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    Ok(result)
}

fn encode_dependencies(c: &mut Canonical, dependencies: &BTreeMap<ElementId, BTreeSet<ElementId>>) {
    c.u64(dependencies.len() as u64);
    for (consumer, dependencies) in dependencies {
        c.str(consumer.as_str());
        c.u64(dependencies.len() as u64);
        for dependency in dependencies {
            c.str(dependency.as_str());
        }
    }
}
