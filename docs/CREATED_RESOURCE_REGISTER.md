# Created-resource register (MP-08 / MP-09 / MP-10 / MP-11, AU-11)

The kernel owns a durable inventory of resources an agent created for a project.
Each row records service, kind, name/URL, optional Vault credential handle,
created-by agent, creation time, revoke method and revocation state. The kernel
records metadata and handles without opening the Vault. Rows survive session and agent deletion.
Listing uses creation order, with resource ID as the stable tie-breaker.

## Tools

- `chariox.mandate.record_resource`: record metadata and a revoke method. A
  `request_id` is scoped to the owner, project and agent. Retrying identical
  metadata returns the original row; conflicting reuse fails with a reason.
- `chariox.mandate.list_resources`: list the authenticated Room's project.
- `chariox.mandate.set_resource_revoke`: supply or change an existing row's
  revoke method while active or failed. It preserves creation provenance.
- `chariox.mandate.revoke_resource`: execute the stored revoke method.

All four tools use one elevation helper, `resource_register_authority`. It
currently resolves a live, task-bound sudo window. This is the integration point
for S1's mandate-or-sudo authority. Arguments cannot select a project or creator.
Leased agents forward to their home kernel through the existing authenticated
lease transport; the home rechecks the lease, exact prompt and elevation. HTTP
revocation checks the actual stored request URL, including after Vault unlock
and immediately before network dispatch, rather than trusting the service label.

## Automatic entries

Generated passwords are recorded through the shared Vault write path. Successful
captured/agent-auth credentials using that path are recorded when their provenance
names a Room and agent. The hook reads metadata only. Generation retries also
repair a missing register row after a storage failure without generating another
password. Successful agent `AddSshMachine` requests record the installation; a
successful `RemoveSshMachine` marks its row revoked. Owner-created machines are
not attributed to an agent.

Automatic rows initially carry an explicit unavailable reason: a generated
password alone does not tell the kernel how to close the account. Set a revoke
request or settings URL once the service provides it. Deleting a local Vault
value is not evidence of external revocation.

## Revoke methods and outcomes

An HTTP method stores a `CredentialHttpRequest` containing a Vault handle,
`DELETE` or `POST`, URL and optional non-secret headers/body. The normal credential
host/use/injection checks apply. The request is bounded to 16 KiB and 30 seconds;
the kernel reads at most 64 KiB of response and returns only status and HTTP code.
Server-controlled bodies and diagnostic errors are never returned by this tool,
even if the service echoes the supplied key. A confirmed 2xx marks the row revoked;
a failure leaves `revoke_failed` with a reason to inspect the service before retry.

A URL method returns `owner_action_required` and the settings URL for the owner
to open. It leaves the row active; merely opening a page never proves revocation.
An unavailable method returns its reason and `ok: false`.

The event and materialized register row commit in one durable-writer transaction.
A compare-and-swap on the complete previous row admits at most one in-flight
revoke and fences a concurrent change to its request. A lost acknowledgement of a
confirmed revoke returns the same confirmed state without another HTTP call.
After a kernel restart an unfinished `revoking` row becomes `revoke_failed`:
the remote effect may have happened, so inspect the service before retrying.
There is no permanently stuck pending state and no automatic repeat of a request.

## Validation

Protocol allocation: local 944 / peer 393. The resource shape and tool names have
a pinned snapshot/hash. Router tests cover elevation, idempotence, credential
links, owner URLs and a service that echoes the injected key; storage tests cover
owner/project isolation, stale requests and restart recovery.

`scripts/e2e-stack/resource-register/live.mjs` drives the production Cloud
`terminal-os` entry on an owned hosted stack with real official Codex/Claude
agents, a hosted WSS relay and a disposable service that issues and revokes keys.
It has a red-on-base mode and
records screenshots, recordings, service results, model IDs, public history, a private
absence check and cleanup outside the repository. It is a development drill,
not real-service AU-11 acceptance; the latter stays NOT RUN until owner accounts
are provided. Do not use the old UI or TUI for its client leg.

The register is agent-facing only. It has no Vault-panel rows or UI proposal,
per the owner's 2026-11-02 decision. Agents list and clean up their resources
through these MCP tools in the new shell's transcript.
