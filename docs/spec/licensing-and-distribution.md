# Licensing and Distribution Policy

**Status:** Normative repository policy; license selection unresolved  
**Reviewed:** 2026-11-11  
**Repository state:** No root `LICENSE`, `LICENSE.md` or `LICENSE.txt` is present in the reviewed `main` revision.

## 1. Current licensing status

Celetixo has **no repository-wide open-source license**. The absence of a license must not be interpreted as permission to copy, modify, publish, sublicense or redistribute the project. Unless and until the rights holder adopts and publishes an explicit license, the project is treated as proprietary/all-rights-reserved to the extent provided by applicable law.

This statement records the repository's current distribution posture. It does not identify the rights holder for every contribution, replace legal advice, or grant rights beyond those established by applicable law and signed agreements.

The repository is currently developed for internal use first. Internal-first is an operating and data-governance choice under ADR-0004; it is not an open-source license and does not itself authorize external distribution.

## 2. Contribution policy is separate

[`CONTRIBUTING.md`](../../CONTRIBUTING.md) already defines Git attribution requirements:

- Git author, committer and co-author metadata must represent human contributors.
- AI tools, coding agents, bots and automated assistants must not be Git authors, committers or co-authors.
- AI-assisted work may be accurately disclosed in engineering records without an AI co-author trailer.
- Local hooks and CI checks enforce recognized automation identities, subject to the documented bypass and identity-recognition limitations.

That document governs attribution and contributor workflow. It is not a software license, copyright assignment, contributor license agreement (CLA), or grant of redistribution rights.

No CLA or contributor copyright-transfer mechanism is asserted by this policy. Any future external contribution program must define the rights and terms governing contributions before accepting them.

## 3. Third-party components and generated artifacts

The absence of a project license does not remove obligations attached to third-party dependencies, copied code, generated artifacts, datasets, benchmarks, fixtures, fonts, documentation or tool binaries.

Before external distribution, maintain an auditable inventory of:
- direct and transitive dependency names, versions and licenses;
- source files or snippets derived from third-party material;
- generated code and generator/runtime licenses;
- benchmark repositories, task suites and fixture provenance;
- vendored tools, container/runtime components and binary redistribution terms;
- notices, attribution and source-offer obligations where applicable;
- any incompatible, restricted or unclear license combinations.

Do not assume that a dependency's license permits redistribution of the whole Celetixo repository or that a public repository's content is unlicensed.

## 4. Distribution restrictions until a license decision

Until a license is selected, approved and committed:

1. Do not advertise Celetixo as open source or assign an OSI-approved license.
2. Do not publish source or binaries under an implied permissive license.
3. Do not accept external contributions on the assumption that `CONTRIBUTING.md` grants the project rights to use them.
4. Do not redistribute bundled third-party binaries without checking their individual terms.
5. Do not publish internal repositories, snapshots, logs, prompts, patches, artifacts or trajectories as part of a software release.
6. Do not treat public visibility of a repository or document as permission to reuse it.

Internal authorized development and use remain governed by the organization's applicable policies, access controls and agreements.

## 5. Required release gate

Before any public source or binary distribution, the project owner must explicitly decide and document:

- the project license and scope of covered material;
- copyright ownership and contributor rights;
- whether a CLA, DCO or other contribution mechanism is needed;
- third-party dependency and binary license compliance;
- NOTICE/attribution requirements and source-distribution obligations;
- treatment of trademarks, names and logos;
- patent grant/retaliation provisions where relevant;
- dataset, benchmark and sample-repository rights;
- security disclosure and vulnerability-reporting process;
- whether any files or modules require separate licenses.

The decision must be reflected in a root `LICENSE` file and corresponding README/release metadata. If different modules or assets have different terms, the boundaries and notices must be explicit.

Legal review is required before selecting terms or making jurisdiction-specific representations. This document is an engineering distribution control, not legal advice.

## 6. Change control

A license selection or change is a material governance decision. Record the approved decision, rights-holder authorization, effective date, covered paths and compatibility review. Do not add a license file based on an assumed default or copy a template without approval.

Until that decision is made, the accurate status is: **proprietary/internal-first development; no public redistribution license has been granted by the repository.**
