# Vault credentials in the user-domain kernel browser

The public multidomain union already includes the MD-5 Vault/controller seam.
This lane strengthens that seam rather than creating another credential service.
It uses local protocol 427 / relay 74 unchanged. The shared MP-11 controller
resolves secure fields in their frame's isolated world, using native DOM
intrinsics rather than page-overridden setters, getters, focus or URL accessors.

## Request and approval

The focused agent loads the kernel-browser MCP tools on demand and calls
chariox.kernel_browser_paste_secret with a credential handle, tab ID, browser
generation, observed document ID and opaque node reference. There is no secret
value argument or result. The password field must still be editable, belong to
the observed document, and have a URL allowed by the credential's browser-use
metadata. A child frame is checked against its own URL, not its parent's origin.

Only the kernel owner can use the host owner's Vault. A collaborator's independent
browser profile cannot resolve those handles. The kernel rechecks the focused
agent admission and current owner after every approval/unlock wait.

Every insertion raises an owner-bound kernel RuntimeInteraction, including when
the Vault is already unlocked. Its existing terminal reply path requires the
owner and a verified Chariox passkey. The operation owner retains the session's
actual identity even when its Vault/profile authority maps a Cloud owner to local. The popup includes the credential handle
and target origin, excluding URL paths and queries. It is not an agent question
and cannot be answered by an agent. It expires after 30 seconds; focus revocation
closes it. The kernel revalidates the exact observed document, frame reference,
URL and owner before proceeding. A navigation during approval cannot deliver.

The human approves through the existing trusted popup. This lane adds no direct
web/TUI credential-fill endpoint; such a UI can be designed separately if needed,
without exposing plaintext in a browser input command.

## Protection before delivery

The user profile reuses RoomSecretObservations with its private profile namespace
and the kernel runtime identity. Under the shared Vault lifecycle lock and the
profile's exclusive observation/input barrier, the kernel registers the
credential source, secret value and exact browser target/document/node binding.
The encrypted durable registry is committed before the host sees a fill action.
All observation paths take the corresponding shared barrier.

The host independently refuses a fill without the matching target/document/node
and protected value. It reserves a one-use delivery before controller dispatch.
The durable binding also refuses another fill into the same document/node after
kernel restart. A failed or lost answer consumes that binding: observe a new
document/field rather than replaying a possibly completed insertion.

The controller resolves the node and owning frame through CDP metadata, enters
an isolated execution world, then rechecks the true password type, attachment,
frame URL and document. Page prototype spoofing cannot supply those checks.
The secret reaches only that allowed field. Vault does not make the receiving
site trustworthy; the credential's allowed-host policy and human approval decide
where delivery is permitted.

## Observation and lifecycle

Screenshot, display, streamed frame and region-capture pixels use the existing
trusted host/frame mask path. They do not ask page JavaScript to draw a mask.
A protection change invalidates cached/delta frames before delivery. Unreadable
registries, unresolved target geometry and document races fail closed with opaque
pixels. Isolated-world notes selection refuses protected text and its context.
Agent-visible snapshots, accessibility/text metadata and diagnostics scrub the
registered value and supported echo variants. PNG data is treated as opaque
already-masked bytes, not scrubbed as textual base64.

Navigation or child-frame replacement invalidates the old target binding.
It cannot authorize delivery into a new document. Old secret echoes remain
suppressed, including after credential deletion and browser/kernel restart;
a new clean field does not inherit the old field's rectangle. App frontend actor
permissions and Room Vault behavior remain on their existing paths. Agent Vault
fill uses the same App-frontend input guard as ordinary agent browser mutations;
an owner popup does not turn that agent into the human App frontend.

## Validation and follow-up

The opt-in kernel-browser native drill exercises synthetic Vault contents,
real Chromium and an encrypted scoped local relay. It verifies one insertion,
owner approval, replay refusal, cross-origin frame policy, masked PNG pixels,
notes receipt refusal, focus revocation and navigation during approval at DPR 1
and 2. The fixture provider is a dev-stub, not a model acceptance test.

Evidence and exact builder commands are outside the repository under
/w/evidence/md-vault/. Rebase/rerun on the coordinator's published
md/stack-on-main is required. The unrelated generic DPR-2 region-mask correction
belongs to mdval and is not duplicated here.
