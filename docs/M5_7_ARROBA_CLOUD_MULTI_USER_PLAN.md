# M5.7 Arroba Cloud Multi-User Bridge

## Goal

Use Arroba Cloud to make multi-user collaboration easy to enter: users log in through device login, invite collaborators to a session, and connect through hosted relay without manually sharing relay credentials.

This milestone is the cloud bridge into the kernel collaboration model. The kernel remains the authority for sessions, workflow authorization, provider ownership, caller-scoped projections, and managed I/O.

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
- workspace links and managed-I/O coordination

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
- managed-I/O conflict decisions
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
- workspace link and managed-I/O coordination

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

- two cloud users accept one session invite and join through hosted relay
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

- cloud-backed session invites work through browser and terminal fallback
- accepted invites produce cloud session membership usable by the kernel
- hosted relay admission is scoped to cloud session membership
- CLI and shell expose invite/member/collaborator commands
- collaborator history is persisted and visible as suggestions only
- live two-user hosted relay drill passes
- M6.5 kernel authorization and redaction rules remain unchanged and covered
