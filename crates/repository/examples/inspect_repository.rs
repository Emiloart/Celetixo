//! Read-only repository intelligence over a live workspace or explicit Git object.
use celetixo_engine::access::AccessSet;
use celetixo_engine::engine::{Engine, InMemoryStore, Permission};
use celetixo_engine::ids::{
    OrganizationId, PrincipalId, RepositoryId, Revision, Scope, WorkspaceId,
};
use celetixo_repository::capture::{CaptureRequest, capture, capture_git_tree};
use celetixo_repository::impact::{ImpactAnalyzer, ImpactRequest};
use celetixo_repository::index::{IndexConfig, Indexer};
use celetixo_repository::reviewed_inputs::add_exact_review;
use serde_json::json;
use std::collections::BTreeSet;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("usage: inspect_repository ROOT [--git-tree FULL_OID] [--exclude PATH] [--changed PATH] [--organization ID] [--workspace ID] [--repository ID] [--reviewed-public-input PATH=CONTENT_DIGEST]...")?;
    let mut oid = None;
    let mut exclusions = BTreeSet::new();
    let mut changed = BTreeSet::new();
    let mut credentials = celetixo_engine::credentials::CredentialPolicy::default();
    let mut organization = "local-inspection".to_owned();
    let mut workspace = "local-workspace".to_owned();
    let mut repository = "local-repository".to_owned();
    while let Some(option) = args.next() {
        let value = args.next().ok_or("option requires a value")?;
        match option.as_str() {
            "--git-tree" if oid.is_none() => oid = Some(value),
            "--exclude" => {
                exclusions.insert(value);
            }
            "--changed" => {
                changed.insert(value);
            }
            "--organization" => organization = value,
            "--workspace" => workspace = value,
            "--repository" => repository = value,
            "--reviewed-public-input" => add_exact_review(&mut credentials, &value)?,
            _ => return Err(format!("unknown or repeated option {option}").into()),
        }
    }
    let scope = Scope {
        organization: OrganizationId::new(organization)?,
        workspace: WorkspaceId::new(workspace)?,
        repository: RepositoryId::new(repository)?,
    };
    let mut request = CaptureRequest::new(root, scope.clone(), Revision(0));
    request.policy.credentials = credentials;
    for path in exclusions {
        request.policy.excluded_paths.insert(
            path,
            "explicit read-only inspection exclusion; relevance remains unproven".into(),
        );
    }
    let captured = match oid {
        Some(oid) => capture_git_tree(&request, &oid)?,
        None => capture(&request)?,
    };
    let mut builder = Indexer::new(IndexConfig::default())?;
    let index = builder.build(&captured.snapshot)?;
    let analyzer = ImpactAnalyzer::new(&captured, &index)?;
    let changed = changed.iter().map(|path| index.files.get(path).map(|file| file.file.clone()).ok_or_else(|| format!("changed path is absent from this snapshot: {path}; use the pre-change generation to inspect a proposed deletion"))).collect::<Result<BTreeSet<_>, _>>()?;
    let query = ImpactRequest {
        snapshot: index.snapshot,
        generation: index.generation,
        changed: changed.clone(),
        transaction_accesses: AccessSet::default(),
        invariants: vec![],
    };
    let impact = analyzer.analyze(&query)?;
    impact.validate_request(&index, &query)?;

    // A local example owns a fresh store and explicitly grants its operator. This
    // is not an authentication service and never accepts a client-supplied graph.
    let contract = analyzer.engine_contract(&captured)?;
    let artifact = contract.digest();
    let (store, administrator) = InMemoryStore::new();
    let engine = Engine::new(store);
    let session = administrator.grant(
        PrincipalId::new("local-inspector")?,
        scope.clone(),
        [
            Permission::Register,
            Permission::ManageModel,
            Permission::Analyze,
        ]
        .into(),
    )?;
    engine.register_repository(&session, captured.snapshot.clone(), contract)?;
    let engine_impact = engine.inspect_repository_impact(
        &session,
        &scope,
        index.snapshot,
        index.generation,
        &AccessSet {
            writes: changed,
            ..AccessSet::default()
        },
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "celetixo/repository-inspection/v1",
            "snapshot": index.snapshot.to_string(),
            "index_generation": index.generation.to_string(),
            "engine_analysis_artifact": artifact.to_string(),
            "capture_consistency": format!("{:?}", captured.report.consistency),
            "capture_complete": captured.report.complete(),
            "capture_census_complete": captured.report.census_complete(),
            "files": index.files.len(), "elements": index.nodes.len(), "edges": index.edges.len(),
            "impact": {
                "id": impact.id.to_string(), "request": impact.request.to_string(),
                "complete": impact.is_complete(),
                "direct_changes": impact.direct_changes.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "direct_consumers": impact.direct_consumers.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "downstream": impact.downstream.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "affected_files": impact.affected_files,
                "verification": impact.verification.iter().map(|check| json!({"id":check.check.to_string(),"kind":format!("{:?}",check.kind),"path":check.path,"reason":check.reason})).collect::<Vec<_>>(),
                "gaps": impact.gaps.iter().map(|gap| json!({"stage":gap.stage,"path":gap.path,"detail":gap.detail})).collect::<Vec<_>>(),
            },
            "engine_impact_complete": engine_impact.analysis.is_complete(),
            "source_mutation_supported": false,
        }))?
    );
    Ok(())
}
