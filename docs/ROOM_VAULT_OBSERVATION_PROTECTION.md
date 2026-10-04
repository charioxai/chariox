# MP-08 / MP-10 / MP-11: autonomous Vault observation protection

Owner decision #8 (2026-10-03) requires autonomous Vault use. Browser and
Computer observations never create a post-insertion clearance interaction.
Computer insertion retains its existing approved-target binding and per-keystroke
focus-change abort. That insertion approval does not authorize later observations.
No managed-placement policy branch is introduced; no MP acceptance item closes.

The home and bound worker register resolved values and target references before
physical input, including input that later fails or is cancelled. Values live in
zeroizing memory and the private sealed registry described below. A private value-free marker fences unknown historical state;
a private sealed registry restores known-value scrubbing and target references
automatically after kernel restart. The registry uses the existing kernel runtime
identity and authenticated encryption with a separate purpose and Room binding.
It creates no new identity or key. A one-time value-free migration permanently
withholds the pre-upgrade history and cached artifacts of Rooms present at the
first upgraded boot. Its persisted history cutoff stays fixed across restarts.
These legacy Rooms immediately permit fresh text, terminal and pixel observations
autonomously, without unknown state or any human action. Rooms created afterward
remain clean across restarts until a secret is inserted.
Unavailable or invalid registries retain their unknown-state fence. No private material is put into diagnostics.

MP-08/MP-11 text scrubbing retains exact values, case variants, URI/JSON/HTML
escaping, Base64/Base64url and hexadecimal forms. The kernel seeds its known
registry into every private controller request, including replacement controllers,
before raw CDP reads or compaction. Raw snapshots, controller responses and events,
kernel tools, MCP results, framed history, Recall and context handoff retain their
shared scrubbing paths. Bound slice provider sessions use their home Room registry.
Unframed terminal bytes and streamed assistant/reasoning remain withheld when a
registry protects that Room: chunk-local matching cannot catch split values.
This requires no human clearance and does not block complete scrubbed tool results.

MP-08/MP-11 fresh worker screenshots use `slice-observation-mask.py`. It re-locates
Browser fields from trusted CDP layout and native fields from the exact approved
X11 control. Browser frame offsets are bound to current document/frame identities.
An isolated world observes browser visibility and scale without page overrides.
Confirmed hidden tabs, departed documents, closed tabs, offscreen regions,
unmapped native controls and Xlib-confirmed destroyed windows contribute no
desktop pixels. The private capture helper returns only confirmed dead native
XIDs to the kernel, which prunes and seals its target list, including when a
subsequent capture fails. Dead controls cannot reappear in policy after restart. Departed document references become echo-only scans of the
current page, including changed isolated frames. Current visible pages are scanned for delayed copies, even
after the original protected tab closes.
The helper compares freshly located regions before and after capturing, masks
them with outward rounding and padding, and only then saves the PNG. Raw captures
exist only in private temporary scratch, which is removed before a result returns.
PNG output carries fresh RGB pixels without copied pre-redaction metadata.

Raw Browser text echoes and opaque canvas/SVG/image/video/frame regions are masked
as well. Browser chrome and link-status regions are masked because titles and URLs
can echo a value outside DOM layout. A replaced top-level input can be re-located
from raw value-bearing input layout without repeating secret insertion. Trusted
`window.innerHeight` includes the horizontal scrollbar when deriving the desktop
content origin. The slice taskbar disables title text and title tooltips.

Unknown regions, mismatched frames or failed capture drop only that observation.
Re-location and fresh capture retry at most three times. Exhaustion returns
`observation redacted, retrying`; observations create no interaction or user wait.

MP-08/MP-11 Vault removal, credential removal, source replacement and value
rotation revoke the affected Rooms' retained values on home and bound workers.
Private sealed values and zeroizing registry memory are removed; an empty sealed
registry and value-free marker retain an unknown-state fence for prior echoes.
Vault-key provenance scopes revocation to affected Rooms and distinguishes
non-Vault sources. Secret input holds a shared lifecycle fence from resolution
through insertion; Vault mutation and Room deletion drain those inputs before
revocation and persistence. Different Rooms can resolve and insert concurrently. Older registries with
no provenance are revoked conservatively only while they retain values. New
credential input cannot narrow that legacy revocation scope. Empty, migrated and
already-revoked registries match no Vault keys and trigger no further revocation.
Before deleting home values, the kernel persists a private value-free revocation
obligation bound to the Room and each slice's creation time and canonical worker
reference. Reuse of a deleted local slice ID cannot fence a fresh physical slice.
Running workers receive a bounded immediate
delivery attempt; a stopped, destroyed or unreachable slice cannot veto the home
credential mutation or Room deletion. Pending obligations survive restart and
Room deletion. Before admitting further provider work, prompts, Browser commands,
Computer observations or display access, the home requires an authenticated
worker acknowledgement on the same Room/slice binding. Delivery resumes
automatically; no human recovery action exists. A pending acknowledgement returns
a distinct `room.secret_observation.revocation_pending` error.
Room deletion removes the home registry and markers after runtime teardown;
cleanup faults keep the tombstone fenced and are logged without reporting an
already-committed deletion as failed. Workers retain only empty unknown-state
protection after acknowledging revocation.

There is no human observation-recovery interaction or clearance fallback.
Vault management does not change Room observation protection. Lost or invalid
secret registries and revoked values still fail closed to protect prior echoes.
This lasting fence returns `room.secret_observation.fenced`, naming the state and
explaining that capture retries cannot clear it. Storage failures name unavailable
protection state. The bounded `observation redacted, retrying` response is reserved
for transient capture/layout failures, not these lasting fences.

MP-08/MP-11 OCR and find-text always re-capture through this masking helper,
including requests carrying old artifact IDs. OCR runs only on a fresh masked PNG,
which is deleted after recognition. Screenshot chunk reads retain the worker epoch
and insertion-revision checks, so pre-redaction and older artifacts cannot be
served. Recovered history and caches stay unavailable; fresh captures do not
reauthorize old content.

MP-08/MP-11 shared local protocol 411 / relay peer 70 are the unshipped G2
protocol pair. `clear_secret_observation` revokes retained values and keeps
observations unknown; it has no owner-clearance flag. Bound workers authenticate
the same home/Room/slice scope as ordinary controller commands and return
`secret_observation_cleared`. Snapshot/hash tests pin the corrected shape within
411/70 under the review-lane instruction; no client minimum changes. Known-value
seeding and sealed registry metadata remain private kernel/controller contracts.

MP-10 focused source and helper checks include false-positive benign observations,
no human interactions, clean restart, autonomous one-time migration and permanent prior-history fencing,
Vault revocation on both home and worker, deferred acknowledgement for stopped
and unreachable slices, deletion cleanup faults, persistent native-XID pruning,
registry Room/identity binding, bounded
retry, old-artifact recapture, trusted Browser geometry, copied text/canvas masking,
and unchanged native focus checks. Real Chromium/X11 helper runs use synthetic
values and preserve benign screen content. Evidence and exact commands stay outside Git.

These checks do not establish a signed aggregate kernel, provider-driven Vault
insertion, complete adversarial rendering coverage, ordinary/Path-1 fresh-machine
parity, or final Browser/Computer acceptance. Arbitrary renderings beyond the
field/text/media/chrome masks require the aggregate security replay. Provider-owned
prior contexts or tools bypassing kernel Browser/Computer APIs are not rewritten.
