# Verified staging and publisher trust

`VerifiedInstallCandidate::from_verified` accepts the package verifier's private
proof type and an enrolled publisher snapshot. It compares the actual public key
that verified the package signature, as well as the exact publisher/key IDs.
Matching manifest claims alone are insufficient. The candidate owns immutable
derived release metadata and its trust snapshot; no raw metadata or `verified`
boolean can be substituted through this API.

`create_and_stage_verified` and `stage_verified` use the kernel's existing SQLite
connection. One immediate transaction rechecks publisher trust, checks the
installation owner, reuses the existing stage/generation/CAS logic, inserts the
signer binding, and commits. First creation, generation allocation, update row,
pending-generation pointer and trust binding roll back together on failure.
The owner is a trusted kernel argument, separate from package metadata.

The internal `app_installation_stage_trust` table records installation and base/
candidate generation, owner, package digest, publisher ID, key ID, enrollment
revision and SHA256 fingerprint of the exact Ed25519 public key. A key revocation
invalidates old snapshots. Explicit re-enrollment increments revision and also
invalidates an older staged operation, even when the key bytes are unchanged.
The installer must abort and restage with current trust, rather than silently
refreshing an old stage's enrollment revision.

`commit_verified` opens one immediate writer transaction, reloads the durable
binding, and calls `StageTrustBinding::require_current` in that transaction. It
checks that the stage is still current and the enrolled snapshot matches both
the registry and the retained binding. It then uses the same activation mutation
as the plain foundation API: capability approval, prepared state and generation
CAS remain required. Active release/schema/capability/catalog/view metadata,
pending pointer, admission state and journal commit or roll back together.

The kernel's typed verified-installation writer adapter accepts immutable
candidates for creation/staging and a stage token for commit. It obtains current
trust on the existing writer connection; the committing transaction still
rechecks that snapshot. The plain metadata `commit` remains for trusted
foundation/test callers, but rejects every stage that has a persisted signer
binding. A verified stage cannot activate through that alternate route. Runtime revocation
and already-running worker termination remain separate supervisor integration.

Bindings follow the existing retained journal. An active generation's binding
survives even when enough aborted updates prune its old journal row. At most the
retained terminal history, one pending candidate and one older active binding
remain. Plain metadata staging APIs remain available for existing kernel/test
foundations; they do not create verified signer bindings.

Release fingerprints use SHA256 over versioned RFC8785 canonical JSON:

- Capabilities includes verified manifest grants, action and information-set
  declarations, and tool-to-action links. It is a content fingerprint; deciding
  which changes require human consent remains kernel policy.
- Catalog includes the full validated declarations. JSON object key ordering and
  whitespace do not change it; declared array ordering is retained.
- View includes the signed UI entry and a sorted path/size/content-digest
  inventory of every verified payload file. This conservatively includes shared
  or dynamic assets outside `ui/`, until the serving contract restricts that set.
- Data schema version comes from signed migrations' target version, or zero when
  the package declares no migrations. No caller-supplied digest/version is used.

Staging persists a candidate; verified commit activates its database generation.
Neither extracts/snapshots files, attests runtime confinement, approves
capabilities, migrates data or launches a worker. The installer must retain the
immutable verified release and complete those external preparation steps before
calling the final trust-fenced commit.

Focused tests use actually signed packages and cover substituted policy keys,
deterministic metadata derivation, shared asset changes, owner/CAS checks,
revocation before stage, revocation/re-enrollment after stage, SQL-triggered
binding-write failure, database reopen and bounded active-binding retention.
Activation cases cover revocation through a second SQLite connection, explicit
re-enrollment refusing to refresh an old stage, unchanged active/pending metadata
on rejection, fresh restaging, approval/preparation checks, missing verified
bindings, rejection of plain commit for current/revoked verified stages, and actual SQL failure after the active pointer UPDATE with rollback
verified after database reopen.
