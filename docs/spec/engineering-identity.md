# Engineering Identity and Snapshot Model

**Status:** Normative subsystem specification  
**Authority:** ADR-0001, ADR-0002 and ADR-0003  
**Purpose:** Define identity contracts for repository snapshots and semantic elements. This document does not select a particular database schema or symbol-index implementation.

## 1. Identity principles

Names, paths, line numbers and display labels are attributes, not durable identity. They can change through rename, move, refactor, formatting, generated output or language-specific transformations.

Celetixo must distinguish:
- identity of immutable content;
- identity of an entity within a snapshot;
- continuity between entities across snapshots;
- confidence that two observations refer to the same logical entity.

A continuity hypothesis is not proof of identity and must not independently authorize safety-critical conflict or commit decisions.

## 2. Identity hierarchy

The model includes:

1. **Tenant/workspace identity:** security and ownership boundary.
2. **Repository identity:** stable repository record independent of remote URL changes.
3. **Snapshot identity:** immutable content-addressed workspace state.
4. **File identity:** file entity within repository history, distinct from its current path.
5. **Semantic element identity:** symbol, type, module, declaration, configuration key or other indexed entity.
6. **Occurrence identity:** a specific reference, call, read, write or syntax occurrence within a snapshot.
7. **Transaction identity:** proposed state transition and its immutable attempts.
8. **Evidence identity:** immutable verification result and artifacts tied to exact inputs.

Every identity is scoped and versioned. IDs must not be recycled after deletion.

## 3. Snapshot identity

A snapshot identifies the exact content participating in a transaction. It includes:
- repository identity and base commit;
- included tracked and untracked files;
- staged and unstaged content as captured;
- editor buffers/overlays;
- relevant submodule/workspace references;
- generated or mounted inputs that participate in analysis or execution;
- snapshot construction policy and manifest;
- content digests and provenance.

The manifest must define inclusion/exclusion rules, canonical path representation, file modes, symlink handling, encoding/binary treatment, submodule treatment and relevant environment/workspace inputs. Two snapshots are equal only when their canonical manifests and content digests are equal under the same versioned canonicalization rules.

A Git SHA remains useful provenance but is not a substitute for a dirty-workspace snapshot identity.

## 4. File and semantic element identity

A file's identity is independent of its path. A rename/move should preserve continuity when repository history and content evidence support it. Copying a file creates a distinct entity unless a specific domain contract establishes otherwise.

Semantic element identity is language-aware and snapshot-scoped. It should capture declaration kind, containing scope, normalized signature/shape and source provenance without relying solely on the symbol's name or absolute path.

The identity system must support languages and non-code artifacts with different semantic models. It must not force configuration keys, SQL objects, generated artifacts and programming-language symbols into a false universal AST abstraction.

## 5. Cross-snapshot continuity

Continuity between old and new entities may use:
- explicit rename/move operations;
- version-control rename/copy hints;
- syntax-tree structure;
- declaration and signature similarity;
- scope and containment relationships;
- reference neighborhood;
- content and provenance history.

The system records the matching method, analyzer/version, supporting evidence and confidence. Ambiguous matches remain ambiguous and may map to multiple candidates.

Heuristic matching can improve navigation, impact discovery and human review. It must not silently collapse identities or authorize a safety-critical OCC decision. If correctness depends on continuity that cannot be established, impact is incomplete and policy applies.

## 6. Occurrence identity

Occurrences are snapshot-specific. A reference occurrence includes its containing semantic element, source span or structural locator, referenced identity when resolved, and analyzer provenance. Line/column positions are presentation coordinates, not stable identity.

When a document changes, occurrence identities are regenerated or mapped with explicit continuity evidence. Stale occurrences cannot be treated as current.

## 7. Identity and conflict analysis

Conflict analysis consumes identity plus access semantics, not identity alone. A stable ID does not establish whether an operation reads, writes, deletes, changes a predicate, or invalidates an invariant.

Every derived access edge must include snapshot ID, index generation, extractor identity/version, resolution status and provenance. Unsupported language regions and unresolved references are represented explicitly.

## 8. Required tests

- Rename and move continuity across supported languages.
- Copy versus rename distinction.
- Same-name declarations in separate scopes.
- Overloads, nested declarations and generated code.
- Ambiguous rename/move matching.
- File deletion/recreation and path reuse.
- Dirty workspaces with staged, unstaged and untracked content.
- Editor overlay changes without Git commit changes.
- Manifest canonicalization and digest stability.
- Stale occurrence/index rejection.
- Identity behavior across parser/analyzer version upgrades.

Identity tests must include adversarial cases where superficial name/path similarity is misleading.
