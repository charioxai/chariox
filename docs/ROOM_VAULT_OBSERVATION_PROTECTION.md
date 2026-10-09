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
Unreadable registries recover as retired-unknown state scoped to that Room. The
kernel logs the recovery without private material, replaces the damaged seal
when storage permits, and withholds its prior history. Unavailable provenance
matches no Vault key, so it cannot veto another Room's Vault mutation. Public
Room deletion remains available. An unknown registry refuses fresh pixel and terminal observations until the existing
verified fresh-environment provisioner clears it autonomously; fresh observations
then flow normally while prior history stays withheld. No human step is added.

MP-08/MP-11 text scrubbing retains exact values, case variants, URI/JSON/HTML
escaping, Base64/Base64url and hexadecimal forms. The kernel seeds its known
registry into every private controller request, including replacement controllers,
before raw CDP reads or compaction. Raw snapshots, controller responses and events,
kernel tools, MCP results, framed history, Recall and context handoff retain their
shared scrubbing paths. Bound slice provider sessions use their home Room registry.
Unframed terminal bytes and streamed assistant/reasoning remain withheld when a
registry protects that Room: chunk-local matching cannot catch split values.
This requires no human clearance and does not block complete scrubbed tool results.

MP-08/MP-11 visual protection follows Miguel's 2026-10-09 fill-target model.
The kernel records the exact browser frame/backend node and document/generation,
or native window/process epoch and AT-SPI object, when it fills a Vault value.
Every capture checks that target again. A still-filled plain input, textarea,
contenteditable or native text entry contributes its rendered box, with small
outward device-pixel padding. Password fields show dots and contribute no mask;
a show-password toggle is checked on every capture and becomes covered.
Navigation, generation change, removal and user clear/replacement retire the
fill target. Registration alone does not mask pixels. There are no page-echo,
container/order/bidi/budget, iframe, media, browser-chrome or whole-window masks.

Fresh worker screenshots use `slice-observation-mask.py` and the shared browser
fill-target collector/native AT-SPI tracker. Field placement is fenced around
capture; a racing/unavailable browser measurement refuses that capture for
retry. Native placement is best effort and creates no fallback window mask.
Raw captures remain in private scratch and are removed before a result returns;
PNG output carries fresh RGB pixels without copied capture metadata.

Unknown regions, mismatched frames or failed capture drop only that observation.
Re-location and fresh capture retry at most three times. Exhaustion returns
`observation redacted, retrying`; observations create no interaction or user wait.

MP-08/MP-11 Vault removal, credential removal, source replacement and value
rotation retire the affected Rooms' active observation values on home and bound
workers. Retired values remain zeroizing, scrub-only entries for that Room's
lifetime, sealed with the existing runtime identity so restart cannot expose old
echoes. They cannot resolve credentials, match Vault keys or authorize insertion.
Known-value text scrubbing continues to protect prior history; visual masks
cover only still-live Vault-filled plain fields under the model above;
rotation introduces no unknown state and the agent's committed credential write
returns its normal scrubbed result. Room deletion removes both active and retired
values. Controller seeds are Room-scoped and released with their controller lease.

Vault-key provenance scopes retirement to affected Rooms and distinguishes
non-Vault sources. Secret input acquires the shared lifecycle fence after approval/unlock waits,
then reloads authoritative credential metadata and Vault configuration, validates
current authorization, and registers current source provenance. It keeps the
fence through resolution and physical insertion; Vault mutation and Room deletion drain those inputs before
retirement and persistence. Different Rooms can resolve and insert concurrently.
Older registries with no provenance retire conservatively only while they have
active values. New input cannot narrow that legacy scope. Empty, migrated and
already-retired registries match no Vault keys and cause no further retirement.

Before changing home values, the kernel persists a private value-free worker
obligation bound to the Room and the physical slice's creation time and canonical
worker reference. Records and delivery locks are indexed per physical slice;
one damaged record or unreachable worker cannot block another slice's admission.
The unshipped flat predecessor records migrate once to that index on startup.
Invalid legacy bindings fail closed during migration. Deleted-slice records are
collected at deletion and reconnect, including records recovered after restart.
Reuse of a local slice ID cannot fence a fresh physical slice.

Running workers receive a bounded immediate delivery attempt. Stopped or
unreachable slices cannot veto a home credential mutation or Room deletion.
Pending obligations survive restart and Room deletion. Before further provider
work, prompts, Browser commands, Computer observations or display access, the
home requires an authenticated worker acknowledgement on the same Room/slice
binding. Slice start, authenticated worker presence and home relay reconnect also
retry delivery independently of Room admission. A pending acknowledgement returns
`room.secret_observation.revocation_pending`. Deleted Rooms upgrade retirement
to a worker wipe; orphaned slice bindings reconstruct a missed wipe after a
teardown interruption. Starting the deleted Room's leftover slice wipes its
sealed observation registry and controller seeds, without admission to that Room.
Home cleanup errors are logged without reporting a committed deletion as failed.

There is no human recovery interaction or clearance fallback. A lost or invalid
registry still fences observation channels and terminal bytes, protecting unknown
prior echoes. Unrelated workflow, messaging, Recall and credential-tool results
continue through known-value scrubbing rather than failing after committed work.
Successful fresh physical provisioning autonomously clears the home/worker fence
and live masking state while retaining `history_before_ms`. This requires
successful inventories proving both old container and home volume absent, no
source slice and no saved-state restore. A stopped container, retained home,
failed inventory, recovery start or metadata-only saved-state reset proves no
such absence and cannot clear the fence. Fresh observations never reauthorize
withheld history or recovered caches. Remaining missing-registry fences return
`room.secret_observation.fenced`; storage failures name unavailable protection.
`observation redacted, retrying` remains reserved for transient capture/layout
failures, not lasting storage fences.

MP-08/MP-11 OCR and find-text always re-capture through this masking helper,
including requests carrying old artifact IDs. OCR runs only on a fresh masked PNG,
which is deleted after recognition. Screenshot chunk reads retain the worker epoch
and insertion-revision checks, so pre-redaction and older artifacts cannot be
served. Recovered history and caches stay unavailable; fresh captures do not
reauthorize old content.

MP-08/MP-11 shared local protocol 411 / relay peer 70 are the unshipped G2
protocol pair. `clear_secret_observation` carries the private authenticated
lifecycle disposition `retire`, `reset_environment` or `delete_room`, and has no
owner-clearance flag. Absent disposition and removed predecessor owner flags
remain retirement. Bound workers authenticate the same home/Room/slice scope as
ordinary controller commands and return `secret_observation_cleared` only after
applying that disposition. Snapshot/hash tests pin all three corrected shapes
within 411/70 under the explicit review-lane instruction; no client minimum
changes. Known-value
seeding and sealed registry metadata remain private kernel/controller contracts.

MP-10 focused source and helper checks include false-positive benign observations,
no human interactions, clean restart, autonomous one-time migration and permanent prior-history fencing,
Vault retirement on both home and worker, deferred acknowledgement for stopped
and unreachable slices, deleted-Room wipe on slice start, per-slice parse/lock
isolation and garbage collection, autonomous fresh-environment reset, deletion
cleanup faults, corrupt-registry recovery through public credential removal and
Room deletion, post-lock metadata removal/host/source/use interleavings,
persistent native-XID pruning,
registry Room/identity binding, bounded
retry, old-artifact recapture, trusted Browser geometry, copied text/canvas masking,
and unchanged native focus checks. Real Chromium/X11 helper runs use synthetic
values and preserve benign screen content. Evidence and exact commands stay outside Git.

These checks do not establish a signed aggregate kernel, provider-driven Vault
insertion, complete adversarial rendering coverage, ordinary/Path-1 fresh-machine
parity, or final Browser/Computer acceptance. Arbitrary renderings beyond the
field/text/media/chrome masks require the aggregate security replay. Provider-owned
prior contexts or tools bypassing kernel Browser/Computer APIs are not rewritten.
