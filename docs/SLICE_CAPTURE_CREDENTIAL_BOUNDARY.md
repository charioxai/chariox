# Slice capture credential boundary

Current save/backup capture is unavailable. The whole-home archive and committed
container layer are not proven to exclude kernel identities, provider profiles,
runtime tokens, container environment secrets or private base-image layers. A
shared preflight refuses capture before commit/tar. Prior saved generations and
credentials are preserved; no migration, key replacement or partial browser-only
save is performed. This affects all current normal local-Docker slices, including
backup/rollback capture. The same existing user-only modes/scopes remain serialized.

This is temporary containment, not completion of Phase1 save/restore. Current
layout labels or environment flags cannot turn it into an accepting guard.

The completing change must verify actual protected persistent mounts for kernel
and official-provider private roots, keep ephemeral tokens outside committed
layers, verify immutable base/configuration boundaries and use a protected durable
archive sink without writable helper-layer staging. Existing mixed homes remain
refused until an explicit non-destructive retention/migration is authorized.
Same-slice restore retains its original protected identities/profiles; future
slices use normal new-worker identity and explicit managed account materialization.
Browser sessions are secret-bearing durable browser data. Workspace is already a
separate host mount and is not newly copied into snapshots.

Acceptance remains pending: real user save/restart with synthetic browser cookies,
localStorage/IndexedDB/CacheStorage/service workers, installed program persistence,
provider/identity exclusion, invalid mount/config/base refusal, previous-generation
preservation on interruption and safe fresh-volume restore. The existing scoped
browser-only synthetic roundtrip is component evidence and does not close this gate.
