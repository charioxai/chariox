# Standalone remote worker SCM enrollment (reserved protocol 397)

Status: design proposal. This document does not change a serialized contract or
claim that standalone SCM transfer is implemented. Protocol 397 is reserved by
the Apps Phase 1 lead for the implementation. The temporary builder PR relay is
an operator workaround and must not become a kernel credential authority.

## Observed gap

Candidate g can approve a standalone machine, create an execution lease, place
an agent with `SpawnAgent.kernel_ref`, and replicate its selected Codex account.
On the Hetzner builder this runs the official Codex CLI with the home-owned
profile `codex-1-6s6cnmim`. Public Git reads work, but authenticated Git pushes
and `gh pr view` fail because the worker has no GitHub credential.

`PrepareManagedEnvironmentGitCredentialEnrollment` (protocol 319) requires a
Cloud-authorized managed environment and its context ticket. The live home
kernel is not connected to Cloud. An ordinary approved remote machine and its
execution lease cannot satisfy that managed-environment authorization.

Relevant existing responsibilities:

- `managed_context/scm.rs` exports the selected `github` credential through the
  source's official `gh` credential lookup and materializes it through `gh auth
  login --with-token` on stdin. It validates selection, rejects replacement of
  unrelated credentials, installs the Git helper, and tracks transactional
  install/removal receipts bound to a managed context.
- `app/remote_lease/provider_account.rs` validates the exact home kernel,
  session, agent, lease, and owner before receiving a provider account.
- `runtime/state/remote_profile_account_runtime.rs` resolves the home-owned
  selection and uses the existing authenticated encrypted peer request path.
- `account_profile_replica*` provides safe directory/file publication and
  rollback. Provider profile storage remains separate from SCM storage.

Copying a token into the existing account replica is insufficient: GitHub is
not a Codex account, the SCM receipt has a different authority binding, and the
provider process needs the correct Git/gh environment. Removing the Cloud
check from managed enrollment would bypass the existing context authority.

## Proposed interface and authority

Add a home-owned, explicit standalone SCM enrollment operation under local
protocol 397. The request selects a home session and agent plus credential IDs;
it accepts no secret, filesystem path, URL, alternate host, or caller-supplied
worker/lease identity. Home derives the current worker and lease from the agent
binding and requires the authenticated authority owner. Default selection is
`none`; placement, provider replication, and profile changes must never imply
SCM consent. Enrollment must be exposed through the shared kernel interface,
not implemented only in `cx`, a web client, or a native TUI.

Home preflights selection and the bound worker's negotiated relay version
before exporting. It uses the existing SCM export routine and sends a typed
SCM materialization over the existing encrypted lease peer transport. The
context binds home kernel, owner, session, agent, execution lease, exact worker,
and a home-generated enrollment generation/idempotency identity. The receiver
must validate the authenticated peer against that context and the active lease;
matching strings inside an unauthenticated payload are insufficient.

The result contains only credential ID/host, generation, safe state/error code,
and an opaque receipt ID. Secret payloads and credential fingerprints stay out
of local replies, projections, audit history, prompts, logs, and Debug output.
The relay and Cloud do not export, inspect, persist, or authorize the credential.

## Worker storage and provider execution

Extract the low-level SCM import/helper transaction into a named module used by
both managed-context enrollment and standalone enrollment. Keep their authority
bindings distinct. Do not manufacture a Cloud context ID to satisfy the current
materializer or weaken its refusal to overwrite another context's credential.

A standalone binding gets a kernel-owned private SCM home scoped to the home
kernel and authority owner, outside repositories, task scratch, evidence,
images, and build outputs. Use directory mode 0700 and file mode 0600, reject
symlinks throughout publication, and preserve unrelated existing worker auth.
A second home or owner must not inherit another binding merely because both
select `github` or execute as Linux root. Same-UID provider processes are not a
security sandbox; this design prevents accidental scope sharing, and operators
must trust the worker host. Hard isolation requires separate execution users or
managed environments.

The provider adapter must compose SCM execution settings with the selected
provider profile. Preserve Codex's profile home and auth; select the SCM home's
`GH_CONFIG_DIR` and a kernel-owned Git config/helper scope for the leased child.
Do not set every provider's `HOME` to a shared root directory or put the token in
command arguments, copied environment snapshots, worktrees, or agent prompts.
Use official Git/gh helpers. Transport secrets only inside the encrypted peer
request and feed native enrollment through stdin.

Make repeated enrollment idempotent after acknowledgement loss and restart.
Persist the binding/install transaction before acknowledging. Rotation requires
an explicit new generation and must not silently replace worker-modified auth.
Lease deletion removes the execution grant/reference, not arbitrary durable
credentials. Explicit owner revoke removes only a receipt-owned credential;
unknown ownership, mismatched content, interrupted removal, or another active
reference must fail closed and preserve durable assets. Revocation cannot
recall a token already copied by a trusted worker process; upstream token
rotation remains an owner action.

## Implementation boundaries and compatibility

1. Shared local request/reply/projection plus home enrollment policy and state.
   Increment the shared local daemon version to **397**, update the protocol
   snapshot/hash tests, and document the contract in `PROTOCOL.md`. Raise a
   web/native minimum only for clients that invoke or depend on this operation.
2. Typed encrypted SCM lease request/reply and worker receipt handling. Reserve
   the next relay peer version with the lead; negotiate it before dispatch,
   reject older and restored incompatible bindings with an upgrade/rebind
   action, and update peer shape/version tests.
3. Shared SCM transaction extraction and leased-child Git/gh environment
   composition, followed by a focused real-relay drill. Managed enrollment's
   existing context authorization and rollback behavior remain covered.

The interface must include enrollment status and explicit revoke; their exact
wire names should be chosen with the implementation and snapshots. Splitting
this into reviewed slices is preferable to adding a payload field without its
ownership, recovery, revoke, or launch-environment behavior. Do not allocate a
new relay version or advertise local protocol 397 from this documentation PR.

## Required validation before replacing the temporary relay

- Real encrypted lease between home and standalone worker, with no Cloud
  connection: enroll selected GitHub auth, run `git ls-remote`, push a throwaway
  `[skip ci]` commit, and run `gh pr view` from a leased Codex agent. Retain only
  safe metadata; close the draft proof PR and remove owned throwaway refs.
- Explicit `none`, unsupported IDs/hosts, unavailable source auth, wrong caller,
  wrong owner/home/worker/session/agent/lease, expired lease, and old peer version
  reject before exporting or publishing.
- Acknowledgement loss, repeated import, home/worker restart, partial native
  enrollment, rollback, concurrent leases, explicit rotation/revoke, and
  worker-modified credentials preserve the correct ownership and durable state.
- Codex account remains authenticated; Claude/native TUI and managed-context
  child environments retain their existing account and permission paths.
- Secret canaries absent from transport diagnostics, history, projections,
  receipts exposed to clients, Debug formatting, workspaces, and evidence;
  traversal/symlink tests cover source-independent destination selection.

Until these pass, builder agents publish through the Mac-side Git/PR relay and
hold no GitHub credentials. The owner decides implementation scheduling and
worker trust/isolation policy; protocol 397's reservation is already approved.
