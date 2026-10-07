# Mirror transport 454 / relay peer 94

This is a validation candidate. It does not satisfy the live staging gate yet.

The isolated observer scans the complete source for observation markers and
registered protected-value echoes before projecting a viewport working set.
Offscreen flow subtrees retain resolved layout placeholders; native scrolling
hydrates visible descendants and removes old offscreen descendants through
ordinary authenticated deltas. Visible fixed/sticky descendants retain their
ancestry even when the flow ancestor is outside the viewport. Hidden subtrees
are omitted only when resolved CSS proves display:none.

Large frames use a private CSS palette and gzip, with 192 KiB encoded credits.
`mirror_next.after_chunk` acknowledges the preceding chunk. A retained frame
repeats identical chunks for retry credits. Every chunk binds subscription, tab,
generation, document and sequence, carries the complete decoded-frame SHA-256,
and declares encoded/decoded lengths. No partial frame is painted or authorizes
input. The client checks lengths, decompression bound, digest, CSS references,
semantic tree hash and normal inert-DOM validation before applying the frame.
A document, policy, source-revision or native custom-shadow change revokes the
retained frame and input epochs. The assembled working set is bounded to 32 MiB.

Resource descriptors deduplicate within a read and reuse admitted content by
SHA-256. Only already-loaded Chromium resource bodies are eligible; private
URLs do not enter the public packet. Bodies revalidate each read, and unused
content is evicted. The resource budget is 16 MiB encoded / 64 MiB decoded,
rather than 128 descriptors. Font bodies missing from the Page cache may use
Network.getResponseBody only for a finished Font response in the current
session/document loader. No additional network fetch is introduced.

A transformed native control keeps source layout/CSS with a separately masked
compositor overlay. It cannot force whole-page fallback. A custom tag is not
sufficient evidence of opaque content: native CDP shadow metadata admits light
DOM, while closed/unknown roots remain protected. Native metadata is rechecked
at frame commit, every chunk credit and input admission. An unavailable hidden
iframe box is ignored only when native CSS proves display:none; a subsequent
visibility change masks the whole capture.

Computed-style reuse requires an exact private CSSOM/media/font/form/custom
state fingerprint. Inaccessible CSS, animation and unobservable dynamic
selectors disable reuse. Protection and geometry still resample every credit.
The private key and raw CSS never enter public mirror packets or logs.

## Limits and outstanding acceptance

This iteration commits complete viewport frames atomically; it does not paint
partially transferred DOM. Traversal, depth, emitted node and visible-text
budgets still fail closed. Very large flat sibling lists can exceed the working
set and need further projection work. Opaque SVG/media regions still use the
existing protected raster path. The approved H.264 stripe implementation has
not been supplied to this branch, so its required replacement of PNG credits
is outstanding. Exact idle refinement and every protection fence remain active.

Source-class Chromium probes are diagnostics, not real built Cloud/kernel/relay
acceptance. The waiting-room progressive-mirror drill in Cloud is the real-path
regression gate. The fixed public-site/DPR/hosted-relay/latency/provider/soak
contract remains required before any DEPLOY_READY.
