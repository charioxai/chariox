# User-domain refusal contract (reservation 434/80)

User-domain denials use the kernel's `DaemonError::UserDomainRefused` and the
existing `KernelTransportError` envelope. Its `code` is `user_domain_<reason>`;
`retryable` is always false and `message` is the constant
`User-domain request refused`. No resource ID, owner, URL, document or existence
information is included. The requesting client already correlates the response
with its request. Runtime traffic follows existing local IPC / browser or TUI ↔
relay ↔ kernel routes; relay forwards errors and never decides access.

The reason vocabulary is defined in one Rust module (`error/user_domain_refusal`)
and one shared OSS client module (`user-domain-refusal`). Cloud mirrors that
client projection in its kernel adapter. Clients expose a typed `refusalReason`
from the bounded code; CLI/browser bridges preserve it together with code and
retryability. Message text, generic errors, malformed frames and unknown codes
never establish an authorization refusal.

| Reason | Meaning |
|---|---|
| `not_focused_agent` | The authenticated local agent is not the current focused agent. |
| `foreign_owner` | An internal admitted identity is mismatched to the requested owner scope; never a global resource existence lookup. |
| `stale_epoch` | A focus, browser or observation-protection epoch changed. |
| `stale_reference` | A document, selection receipt or subscription reference is no longer usable. |
| `not_granted` | This owner/actor has no usable reference or capability, including nonexistent and foreign IDs. |
| `sensitive_requires_focus` | Generic input cannot authorize a sensitive field; the privileged focus/Vault path is required. |

Host policy refusals are branded `UserDomainRefusal` instances. Arbitrary page,
CDP and transport exceptions cannot impersonate them with a matching message or
code. Trusted document-check errors retain stale-reference attribution. The
Rust host RPC adapter preserves only recognized codes as typed failures; other
controller and I/O failures remain generic transport errors. Kernel focus,
actor and note-store policy failures use a bounded internal compatibility
adapter; raw controller messages never pass through that adapter.

Nonterminal public frontend routes are refused before session-grant lookup, so
revoked grants, transport peers and unauthenticated callers receive the same
bounded admission error. Agents retain the existing focused MCP admission path.

Foreign and fabricated tab/view/note references return the same `not_granted`.
No global lookup asks whether the foreign resource exists. Owner-scoped lists
remain filtered projections, not errors. App package/storage/tool failures keep
their own codes; only user-view admission failures are normalized. Room browser
controller behavior is unchanged; Room membership refusals during user-owned
notes are attributed at the kernel notes boundary.

Reservation overlap: lane mdaccess is building grant holders/revoke/sensitive
access under local432. This lane adds refusal attribution under local434/relay80
and does not introduce grants or change their policy. `not_granted`,
`not_focused_agent`, `stale_epoch`, `stale_reference`, `foreign_owner` and
`sensitive_requires_focus` are the proposed shared vocabulary; no remote agent
for mdaccess appeared in this runtime session, so the coordinator contact route
was requested while implementation continued. Fold the reason module and
protocol minimums into one union step when combining the lanes; do not maintain
two competing vocabularies. The existing error envelope fields are unchanged,
but bounded codes/transport semantics change, so the Protocol Change Rule is
applied with 434/80, explicit reason/error snapshot and SHA256 guard. Legacy427
note snapshots remain intact. Clients that depend on these denials require434.

Validation: nine individually named host/client fail-first cases, structured
code/bridge tests, non-refusal lookalike tests, protocol snapshot/hash checks,
focused Rust, native Room/user/App drill and the complete55-case live matrix
with both production TUIs. Evidence and commands live outside repositories in
`/w/evidence/md-refusals/`. Release acceptance still needs a fresh approved signed
runtime: public runtime revision5 lacks the group-mapping handshake.
