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
tooling, not a documented provider export RPC. See the lane's retained real
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
A published checkpoint remains available for an explicit retry if the provider
probe fails; import does not admit a user turn or change the committed route.

Local protocol **601** adds optional `native_checkpoint_binding` to the existing
provider-launch request. It carries safe checkpoint metadata, not native history.
Callers must check protocol 601 before using it; older kernels ignore unknown
fields. The corresponding private `RequiredNativeSession` persists until the
first accepted native turn. Launch rejects missing or mismatched native state,
and resume failure neither clears that state nor substitutes another provider.
A successful official resume and accepted turn retire the constraint, preserving
ordinary recovery behavior for subsequent work. Existing calls without a binding
retain their previous behavior. No peer message changes in this PR, so reserved
peer allocation 207 is unused; existing remote-native forwarding refuses the
unsupported checkpoint/interface combination before sending anything.

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
and managed lifecycle parity, peer fencing, Mac directions, full workspace
transfer, UI and sustained Move drills remain separate acceptance cells.
