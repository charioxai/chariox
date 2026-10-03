# MP-08 / MP-10 / MP-11: autonomous Vault observation protection

Owner decision #8 (2026-10-03) requires autonomous Vault use. Browser and
Computer observations never create a post-insertion clearance interaction.
Computer insertion retains its existing approved-target binding and per-keystroke
focus-change abort. That insertion approval does not authorize later observations.
No managed-placement policy branch is introduced; no MP acceptance item closes.

The home and bound worker register resolved values and target references before
physical input, including input that later fails or is cancelled. Values live in
zeroizing memory. A private value-free marker fences unknown historical state;
a private sealed registry restores known-value scrubbing and target references
automatically after kernel restart. The registry uses the existing kernel runtime
identity and authenticated encryption with a separate purpose and Room binding.
It creates no new identity or key. Legacy, unavailable or invalid registries do
not grant observation access. No private material is put into diagnostics.

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
Confirmed hidden tabs, offscreen regions and unmapped native controls contribute
no desktop pixels. Current visible pages are scanned for delayed copies, even
after the original protected tab closes.
The helper compares freshly located regions before and after capturing, masks
them with outward rounding and padding, and only then saves the PNG. Raw captures
exist only in private temporary scratch, which is removed before a result returns.
PNG output carries fresh RGB pixels without copied pre-redaction metadata.

Raw Browser text echoes and opaque canvas/SVG/image/video/frame regions are masked
as well. Browser chrome and link-status regions are masked because titles and URLs
can echo a value outside DOM layout. A replaced top-level input can be re-located
from raw value-bearing input layout without repeating secret insertion.

Unknown/stale regions, mismatched frames, missing controls or failed capture drop
only that observation. Re-location and fresh capture retry at most three times.
Exhaustion returns `observation redacted, retrying`; it creates no interaction,
clearance callback or user wait. A permanently unlocatable target remains unavailable
for that observation. Legacy state with no recoverable registry is similarly
unavailable; this is a safe observation drop, not a human approval path.

MP-08/MP-11 OCR and find-text always re-capture through this masking helper,
including requests carrying old artifact IDs. OCR runs only on a fresh masked PNG,
which is deleted after recognition. Screenshot chunk reads retain the worker epoch
and insertion-revision checks, so pre-redaction and older artifacts cannot be
served. Recovered history and caches stay unavailable; fresh captures do not
reauthorize old content.

MP-08/MP-11 shared local protocol 378 / relay peer 69 remain unchanged. Their
legacy `clear_secret_observation` command is rejected; it cannot clear protection.
No shared serialized shape or client minimum version changes. Known-value seeding
extends only the private kernel/controller stdio envelope. Protocol hash tests
continue to bind the shared shapes.

MP-10 focused source and helper checks include false-positive benign observations,
no human interactions, registry restart/Room/identity binding, stale-region bounded
retry, old-artifact recapture, trusted Browser geometry, copied text/canvas masking,
and unchanged native focus checks. Real Chromium/X11 helper runs use synthetic
values and preserve benign OCR. Evidence and exact commands stay outside Git.

These checks do not establish a signed aggregate kernel, provider-driven Vault
insertion, complete adversarial rendering coverage, ordinary/Path-1 fresh-machine
parity, or final Browser/Computer acceptance. Arbitrary renderings beyond the
field/text/media/chrome masks require the aggregate security replay. Provider-owned
prior contexts or tools bypassing kernel Browser/Computer APIs are not rewritten.
