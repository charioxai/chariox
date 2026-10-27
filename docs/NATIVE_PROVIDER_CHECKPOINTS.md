# Native provider checkpoints

MP-08 / MP-09 / MP-10 / MP-11: this is the provider boundary for PR 1a of the
Move plan. It does not expose a Move action, replace an execution binding, copy
workspaces, or implement the retirement fence. Those remain later bounded PRs.

The verified format is the selected Codex CLI **0.159.3** session JSONL file,
reported by official `thread/read.path`. The file stays opaque. The adapter
never parses a provider database, copies an account root, rewrites history, or
supplies a conversation transcript as a new prompt. Initial support is the
Chariox Codex interface on Linux → Linux. Other providers, native TUI interfaces,
operating-system pairs and CLI versions return typed unsupported reasons until
their own real drills establish support.

The official [app-server documentation](https://learn.chatgpt.com/docs/app-server)
describes native JSONL persistence, `thread/read`, `thread/resume` and versioned
schema generation. The [CLI reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli)
describes `codex exec resume`; its Markdown export is not a native checkpoint.
The selected-file portability claim is an inference verified with official
tooling, not a documented provider export RPC. See the retained real
proof for its exact CLI, model, account-transfer and workspace-relocation results.

`ProviderProcessServiceStore::export_native_checkpoint` requires an ended run
and no remaining runtime binding or prompt I/O. Official native metadata must
also identify the expected idle session. The adapter checks the provider's
authenticated identity, captures one stable, bounded native file, and binds its
digest, model, effort, CLI version and source platform. Anonymous/API-key account
states without a provider-reported identity are unsupported. The existing
provider-account transfer remains independent and excludes session stores.

A `ProviderNativeCheckpoint` is private artifact data. Its debug projection
contains only format, byte count and digest. A generic `LaunchProviderRequest`
does not serialize the payload. Future lease composition must carry it through
protected encrypted artifact transport and preserve the existing owner/lease
admission checks; receiving bytes is not permission to execute.

Target import verifies the same authenticated account and launch configuration,
checks hashes and size, stages a mode-0600 file through pinned no-follow directory
handles, and publishes without overwriting another file. A matching existing
artifact is reusable; divergent state is refused. Temporary files are removed
on either outcome. The target's official resume must return the same native ID,
model and relocated working directory before the adapter returns readiness.
Import integrity requires the exact exported bytes. Before the first user turn,
continuity verification accepts official provider appends only while the entire
imported checkpoint remains an unchanged prefix at a JSONL record boundary.
Truncation or rewritten imported history still fails. Codex `thread/resume`
appends a native event even before a user turn; treating its whole file as an
immutable artifact would reject a successful native resume. Account, native ID,
configuration, bounded file reads and no-fresh-session safeguards still apply.
A published checkpoint remains available for an explicit retry if the provider
probe fails; import does not admit a user turn or change the committed route.

Local protocol **637** adds optional `native_checkpoint_binding` to the existing
provider-launch request. It carries safe checkpoint metadata, not native history.
Callers must check protocol 637 before using it; older kernels ignore unknown
fields. The corresponding private `RequiredNativeSession` persists until the
first accepted native turn. Launch rejects missing or mismatched native state,
and resume failure neither clears that state nor substitutes another provider.
A successful official resume and accepted turn retire the constraint, preserving
ordinary recovery behavior for subsequent work. Existing calls without a binding
retain their previous behavior. Durable Move carries protected artifacts through
the authenticated 638/240 peer contract described below; generic client launch
requests continue to carry metadata only.

The ignored `live_codex_native_checkpoint_driver` test exports/imports private
artifacts for a real two-kernel/TUI drill. It accepts an explicit private manifest
through `CHARIOX_NATIVE_CHECKPOINT_DRILL`; it prints no payload or credentials.
Its source input must include the real kernel's ended-run receipt. Ordinary
focused tests cover corrupted artifacts, path confinement, symlinks, size bounds,
non-overwriting publication, private projections and required-resume recovery.
Real acceptance additionally requires actual source and target kernels, their
product-linked account transfer, the real TUI, a relocated workspace and a next
provider turn that recalls previous context. A helper or fixture alone is not
MK01 acceptance.

No managed-only provider branch or shutdown policy is added. Integrated ordinary
and managed lifecycle parity, Mac directions, route UI and sustained Move drills
remain separate acceptance cells.

## Durable Move and Return

MP-08 / MP-09 / MP-10 / MP-11: protocol 638/240 composes this adapter with
Project transfer. The home kernel journals one generation per agent. It fences
prompt and provider admission, confirms source teardown, and captures the native
checkpoint together with the current Project. Encrypted peer requests carry
bounded private chunks; clients receive no native payload.

The target imports into a new unused root and returns a receipt identifying the
primary workspace and all supporting workspaces, including directories. Its
agent must use that primary path. Native import and the official session probe
finish before the home operation commits and retires its source. A readiness
probe admits no user turn. The next real provider launch retains the complete
checkpoint binding until native continuity is accepted.

Return captures the current target Project again and creates a new Project
identity and unused workspace root. It reuses the original home account
profile. Native import extends the stopped home rollout only when every old
byte is a complete-record prefix of the incoming history; divergent history
is preserved and refused. Resolve that divergence before retrying, or cancel
the pending Move until its target activates. Cancelling retires the target's
lease and backing agent, restores the agent's previous route and workspace, and
rewinds the target's native thread to exactly the imported checkpoint, dropping
only the readiness probe's appends (no user turn can have run there). A retry
after the source has advanced therefore extends that thread. If the thread is no
longer an extension of the checkpoint it is left untouched, the cancellation
still completes, and the retry reports the divergence. Ordinary checkpoint import
remains strict and accepts only its required original prefix plus native
appends from the readiness probe, including an unindexed rollout.

The original home checkout and previous workspace materializations remain
available. Return never resets or merges them. Home Room identity, history
and authority stay at home. Only the agent's execution workspace changes.

Private journals recover independently of periodic Room snapshots. An
acknowledged worker backing restores idle with its original lease timestamps
and caller bindings, without starting a provider run. An unacknowledged empty
lease can be recreated from the same materialization receipt. Home recovery
retries the same generation under the admission fence. Before binding, a
recoverable failure can release the source and roll back. After commitment,
recovery finishes target activation and source retirement. An unavailable peer
leaves the operation fenced for retry after reconnection.

A source with active provider work or a live leased process wake must finish or
cancel that work first. If a restart lost the source run before its checkpoint
was captured, the operation rolls back; resume the source for one completed
turn before retrying. A stored, still-required checkpoint also supports an
immediate Return before the first target turn. Scope remains the managed Codex
adapter's supported CLI version and platform contract.
