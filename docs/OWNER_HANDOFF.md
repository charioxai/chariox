# MP-08 / MP-10 / MP-11 — protected owner hand-off (A07, protocol 477)

An agent with an owner-granted user-domain browser can request one owner action
with `chariox.handoff.request`. The request names a freshly observed tab,
browser generation, document and node, an explanation, a reason (`model_refusal`,
`automation_disallowed`, `human_verification` or `owner_authorization`) and a
kind (`click`, `code` or `secret`). Optional keep/add/remove lines describe the
intended change; the owner verifies those lines before authorizing the action.
A refusal never authorizes the agent to retry or bypass the refused step.

The kernel creates a `hand_off` obligation and a session kernel-operation
interaction, projected through the same local/remote TUI and web paths. The
agent yields on the returned `completion-<obligation_id>` registration and ends
its turn. It must re-read the actual service result after waking, compare it
with the intended change and report missing or extra changes, including a
missing SSH rule in a firewall change. Owner completion is a safe status,
not independent proof that the service accepted the intended change.

The owner's protected answer is `RespondToHandoff` with session and interaction
IDs and one action: `click`, `enter_value`, `done` or `cancel`. Code/secret
hand-offs require protected entry; `done` is available only for a click completed
through the existing browser attachment. Values cannot be custom replies,
provider answers, ordinary prompts, command metadata or cached/replayed input.
The first owner answer is durably claimed before input. Other terminals lose
the action and late answers are refused, including after restart. Secret/code
values are registered for observation protection before insertion; inputs,
observations, screenshots and errors use the existing protected browser path.
Only a secret hand-off may offer separate owner-selected Vault storage; it grants
no model read capability. A failed save is reported separately from successful
field entry. Vault-save acceptance also depends on the PR6 integration.

Physical input revalidates the requesting resource grant, task cancellation,
tab/generation/document/node, field kind and full document URL under the capture
barrier. Terminal projections contain only origin/path and a bounded label; a
private durable URL fingerprint catches query/fragment changes without storing
them in the interaction. A stale binding requires a fresh request. An uncertain
input is never repeated automatically. A pending unclaimed request survives
restart only for its remaining absolute expiry window; a claim without a retained
outcome becomes `uncertain`. Timeout and cancellation deliver safe durable source
outcomes. They never approve, replay or create a second browser.

Local and remote TUIs expose the request in the Chariox approval panel (F8).
Selecting protected entry masks the value, consumes its keys/paste before prompt
handlers, clears it on submit/dismiss/disconnect and sends the dedicated request.
Web uses the trusted terminal popup, with a read-only view of the existing bound
tab and an explicit owner action. Click hand-offs also expose Take over for the
existing tab. Unsupported/opaque content uses that attachment and Done after
the owner acts. No extra browser profile or navigation is created by attachment.

Focused source/component checks are regression evidence. A07 acceptance still
requires the real service/refusal/challenge, real official providers, built web
at DPR1/2, local and remote TUI, hosted WSS, realistic network and duration,
private absence checks, service outcome verification and owned cleanup.
