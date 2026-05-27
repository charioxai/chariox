# M5.7 Arroba Cloud Multi-User Bridge

## Goal

Use Arroba Cloud to make multi-user collaboration easy to enter: users log in through device login, invite collaborators to a session, and connect through hosted relay without manually sharing relay credentials.

This milestone is the cloud bridge into the kernel collaboration model. The kernel remains the authority for sessions, workflow authorization, provider ownership, caller-scoped projections, and workspace live sync.

## Relationship To Adjacent Milestones

M5.6 is closed. It delivered hosted relay onboarding:

- browser/device login
- kernel-owned cloud profile state
- kernel-owned hosted relay pairing
- kernel-owned runtime-token and client-token minting
- logout
- live cloud relay drill coverage

M6.5 covers kernel collaboration semantics:

- session-scoped membership
- owner-only providers and freeform agents
- shared workflow graphs
- public node labels
- endpoint ownership
- caller-scoped redaction
- stale workflow edit rejection
- workspace links and workspace live sync coordination

M5.7 connects those two pieces through Arroba Cloud.

## Product Flow

1. User A logs in with `/relay cloud login`.
2. User A creates or opens a local Arroba session.
3. User A runs a cloud invite command for that session.
4. Arroba Cloud returns an invite URL/token.
5. User B opens the invite URL, logs in or registers, and accepts.
6. User B's local CLI/kernel receives cloud identity and session membership context.
7. User B joins the shared session through hosted relay.
8. From that point, M6.5 kernel rules apply.

## Cloud Responsibilities

Arroba Cloud should own:

- cloud users and accounts
- device-login cloud sessions
- session invite tokens
- invite accept/revoke/expiry/max-use state
- collaborator history
- hosted relay credential issuance for invited members
- route admission for hosted relay connections

Arroba Cloud must not own:

- workflow graph authorization
- provider or model visibility
- prompt visibility decisions
- endpoint execution authorization
- workspace live sync conflict decisions
- kernel session mutation semantics

## Kernel Responsibilities

The kernel should own:

- local session authority
- cloud user identity attached to local caller identity
- session membership projection
- owner-only freeform agent/provider visibility
- workflow ownership checks
- endpoint owner checks
- prompt and endpoint prompt redaction
- stale workflow revision rejection
- workspace link and workspace live sync coordination

## Required Cloud API Surface

Proposed minimal API:

- `POST /sessions/:sessionId/invites`
- `GET /sessions/invites/:inviteToken`
- `POST /sessions/invites/:inviteToken/accept`
- `POST /sessions/:sessionId/invites/:inviteId/revoke`
- `GET /sessions/:sessionId/members`
- `GET /collaborators/recent`
- `POST /relay/session-token`

All write requests require a valid cloud session token from device login.

## Required Kernel/CLI Surface

Slash and shell command families should match:

- `/cloud invite create [--expires ...] [--max-uses ...]`
- `/cloud invite accept <invite-token-or-url>`
- `/cloud members`
- `/cloud collaborators`

The CLI should keep browser-first behavior where useful:

- if a browser is available, open the invite acceptance URL
- otherwise print the URL/token and continue with a terminal fallback

## Current Implementation Status

- Cloud service API and Prisma persistence are implemented in `arroba-cloud`: shared-session invites, members, invite expiry/max-use handling, creator-only revoke, and collaborator history are available.
- OSS kernel IPC now exposes cloud session invite create/show/accept/revoke, cloud session member listing, and recent collaborator listing through the persisted cloud relay session token.
- TUI slash commands are wired:
  - `/cloud invite create [max-uses|--max-uses n]`
  - `/cloud invite accept <invite-token-or-url>`
  - `/cloud members`
  - `/cloud collaborators`
- `arroba-shell` shared executor supports:
  - `cloud invite create [max-uses]`
  - `cloud invite accept <cloud-invite-token> [local-invite-token]`
  - `cloud members`
  - `cloud collaborators`
  - `cloud status`
- The first bridge uses a paired cloud invite token plus the existing local kernel session invite token. This keeps kernel authorization unchanged while allowing cloud acceptance to establish cloud membership and collaborator history.
- Hosted relay token issuance now requires an active cloud session token. Session-scoped client tokens require accepted cloud shared-session membership and bind the relay subject to the logged-in cloud session's client id.
- The live cloud relay drill now covers three cloud users: invited users accept the cloud invite, receive session-scoped hosted relay tokens, join the kernel session through the relay with the paired local invite, and run the M6.5 workflow authorization/redaction assertions through that cloud path.

## Data Model

Cloud needs durable tables for:

- cloud session invite
- cloud session member
- collaborator contact or recent collaborator
- relay session credential issuance/revocation

The kernel needs persisted or projected fields for:

- cloud user id
- cloud account id
- cloud session membership id
- cloud session invite mapping to local session id

## Security Rules

- Invites are per session.
- Invite tokens are opaque, expiring, and revocable.
- Hosted relay credentials are scoped to account/session/caller.
- Collaborator history never grants access.
- Self-hosted relay remains identity-less in OSS unless the operator builds their own identity service.
- All user-generated payloads still use the existing relay encryption model.

## Live Drills

Required before closing M5.7:

- two cloud users accept one session invite and join through hosted relay: **covered by `pnpm --filter @arroba/cli run cloud-relay:drill`**
- user B cannot see user A's freeform providers or agents
- user B can see user A's public workflow node label
- user B can add an edge involving one of B's own nodes and user A's node
- user B cannot run user A's endpoint
- user A can run user A's endpoint
- workflow changes live-update in both CLIs
- stale workflow revision rejection still works
- collaborator history suggests user B in a later session but does not auto-add user B

## Exit Criteria

M5.7 is complete when:

- cloud-backed session invites work through browser and terminal fallback: **implemented for CLI/kernel flows** through generated invite URLs and terminal token fallback; the hosted web acceptance page remains cloud/web product work.
- accepted invites produce cloud session membership usable by the kernel: **implemented**; cloud acceptance returns the cloud user id and session membership, and CLI/shell paths can join the local kernel invite when the local token is present.
- hosted relay admission is scoped to cloud session membership: **implemented at token issuance**; Cloud only mints session-scoped client relay tokens for accepted shared-session members.
- CLI and shell expose invite/member/collaborator commands: **implemented**.
- collaborator history is persisted and visible as suggestions only: **implemented at API/command level**.
- live two-user hosted relay drill passes: **implemented**
- M6.5 kernel authorization and redaction rules remain unchanged and are covered both by the original M6.5 drills and by the cloud relay hardening path using cloud-issued session-scoped relay tokens.
