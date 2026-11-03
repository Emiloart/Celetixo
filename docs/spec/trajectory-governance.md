# Trajectory Data Governance

**Status:** Normative data-governance specification  
**Authority:** ADR-0001 and ADR-0002  
**Purpose:** Define controls for collecting, retaining, evaluating and using engineering trajectories. Storage does not imply permission to train.

## 1. Governing principle

A trajectory is a structured record of an engineering task and its execution, not an unfiltered transcript. Collection, retention, evaluation use and model-training use are separate decisions with separate eligibility checks.

No repository content, patch, prompt, tool output or trajectory may be used for training merely because Celetixo can technically access or store it.

## 2. Provenance and lineage

Every trajectory must retain, where applicable:
- source repository and owner/tenant;
- source acquisition method and timestamp;
- applicable license and version;
- authorization/consent basis and scope;
- snapshot, task, transaction and attempt identities;
- model/provider/version and relevant configuration;
- prompt/template and tool-policy versions;
- tool calls and worker actions;
- patch digest and execution environment;
- checks, results, artifacts and impact provenance;
- retries, rebase, rollback and human decisions;
- dataset membership, transformation and export lineage.

Lineage must allow a derived dataset item to be traced back to its source and the governing eligibility decision.

## 3. Eligibility gates

Before collection or downstream use, determine:
1. **Rights:** license, contractual permissions and repository owner authorization.
2. **Purpose:** whether storage, evaluation, analytics and training are separately permitted.
3. **Privacy:** personal data, confidential material and applicable data handling obligations.
4. **Secrets:** credentials, tokens, keys, certificates and sensitive configuration.
5. **Security:** malware, exploit payloads or other content requiring restricted handling.
6. **Quality:** whether the trajectory has complete and trustworthy state/evidence.
7. **Contamination:** overlap with evaluation tasks, hidden tests or benchmark solutions.
8. **Representativeness:** language, repository, task and outcome distribution.
9. **Retention:** retention period, deletion obligations and backup/export behavior.

Unknown rights or purpose are not equivalent to permission.

## 4. Consent and repository authorization

The product must define explicit repository onboarding terms and authorization controls before accepting third-party repository data. User authorization to execute a task does not automatically grant permission to retain source content indefinitely or train models on it.

Where consent or contractual scope is required, record the scope, actor, timestamp, policy version and revocation mechanism. Revocation must propagate to eligible stored copies and derived datasets according to documented legal and technical limits.

## 5. Data minimization and secret handling

Apply secret detection and redaction before broad access, analytics export or training eligibility. Preserve the original only when the retention basis and access controls permit it. Redaction must be recorded and must not silently alter evidence used to validate a transaction.

Use least-privilege access, encryption in transit and at rest, scoped credentials, audit logging and separation between tenant data. Do not place secrets in model prompts or training datasets.

## 6. Retention, deletion and access

Define retention by data class: source snapshots, patches, logs, model prompts, tool outputs, verification artifacts, analytics and derived datasets may require different policies.

Support:
- tenant-scoped access and export;
- retention expiry;
- deletion and revocation workflows;
- legal hold where applicable;
- backup lifecycle and deletion propagation;
- audit records for access and administrative action.

Deletion from a primary database alone is not sufficient if replicas, object versions, caches, analytics tables or derived datasets retain the material.

## 7. Evaluation isolation

Benchmark tasks, hidden tests, reference solutions and evaluation labels must be isolated from training and prompt/context retrieval. Dataset construction records split policy and contamination checks. Evaluation results must not leak into worker context in a way that invalidates the benchmark.

## 8. Training eligibility

A trajectory is training-eligible only after a versioned policy decision confirms:
- adequate rights and purpose;
- privacy and secret controls;
- acceptable quality and provenance;
- no prohibited evaluation contamination;
- documented transformations and lineage;
- permitted retention and deletion handling.

Training eligibility is revocable where required. Model/dataset release records must identify source dataset versions and applicable restrictions. Do not assume a trained model can always be made to forget individual examples; this limitation must be part of consent and product policy.

## 9. Required governance artifacts

Before production ingestion of external repositories, define:
- terms and repository authorization flow;
- license/rights classification;
- consent and revocation records;
- privacy and secret-handling policy;
- data classification and access matrix;
- retention/deletion schedule;
- evaluation/training separation controls;
- dataset approval and audit process;
- incident response for unauthorized or sensitive data.

Legal review is required for jurisdiction-specific obligations. This specification is an engineering control framework, not legal advice.
