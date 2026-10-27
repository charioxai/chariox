# Browser secret insertion uses the home Room route

Controller-backed `paste_secret_to_slice` now follows the existing Room browser
forwarding path. A leased worker sends only the opaque credential handle and
target metadata; the home kernel validates the relay sender, lease, bound
worker, reserved Room slice, agent membership, masked field, target document,
and credential scope before it can unlock the vault or fill the field.

The home creates the credential service again after the vault interaction. This
prevents deletion, revocation, or narrowed scope during an awaited unlock from
being bypassed by a pre-interaction metadata snapshot. The resolved value is
held only as zeroized process-local input for the attributed Browser locator
action; it is not returned in the relay response or runtime-tool payload.

The Room browser allowlist admits only this browser-secret tool and its existing
aliases. Other credential tools remain on their established home-authority
paths. Worker forwarding reserves 345 seconds for the existing 300-second vault
interaction, browser action, and response buffer without reducing a longer
configured timeout. No serialized request, response, or protocol version
changed.

## Validation

The focused kernel tests cover alias admission/rejection and the worker timeout
for a pending vault unlock. Full acceptance still requires matching rebuilt
home/worker binaries and image, the official-provider onboarding flow, local and
remote TUI receipts, live vault unlock/revocation, wrong-origin and locked-vault
rejection, and leak scans. Those checks require the managed runtime and external
provider credentials; source-only checks do not close them.
