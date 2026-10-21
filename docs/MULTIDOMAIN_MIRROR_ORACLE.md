# MP-08 / MP-10 / MP-11 — mirror M0 oracle

The frozen runtime remains OSS `45e365626` (protocol 517) paired with Cloud
`c8bd439e9`. Oracle changes stack separately on those heads and do not change
runtime contracts or fallback policy. Whole-page protected VIDEO remains the
agreed fallback; region video and first-paint work belong to later milestones.

`packages/kernel-client/browser-mirror-oracle.mjs` is the independent coverage
and drift calculator. Cloud's `scripts/e2e-stack/mirror-oracle.mjs` is an exact
copy. Both runners use the same uncapped `DOMSnapshot.captureSnapshot` contract,
including visible child documents, text fragments and computed colors. They do
not use the product snapshot's 5000-node projection or the mirror observer's own
coverage claim. No node cap applies to the denominators; only diagnostic examples
are capped. Missing or capped native evidence cannot qualify a row.

- **C_area:** one minus the exact union of unmatched native visible paint boxes
  divided by the union of all native visible paint boxes. Box overlap is counted
  once. Text, replaced elements, backgrounds and borders are paint primitives.
- **C_text:** matched native visible nonblank text-node occurrences divided by
  all such occurrences. Equal strings do not collapse duplicate occurrences.
- **Drift:** maximum matched box/text-fragment error, missing paint, fragment-count
  changes and exact text-color mismatches. The gate is 0.5 CSS px with no missing
  paint, changed fragment count or changed color.
- **Raster:** RGB mismatch outside a source-only one-native-pixel edge band,
  threshold 0.10%. Missing paint cannot enlarge that exclusion band.

Cloud's shared public-site step requires an observed kernel protocol >=517,
subscribes with `wire: 2`, waits for and validates the first reset packet, and
reads `data-paint-mode`. Restored/watched tabs use the ordinary **New tab** button;
input to watched tabs is never forced. A VIDEO/IMAGE fallback has no qualifying
DOM paint: its coverage is zero and the four-metric report fails. The actual
fallback screenshot remains measurable independently.

Native snapshots use a drill-only read-only CDP attachment to the disposable,
kernel-owned public browser. CDP never dismisses banners, dispatches input,
injects DOM or navigates. Native screenshots use the protected product capture
route. That route has a fixed DPR2 output. The oracle projects those protected native
pixels bilinearly into the actual, pixel-aligned viewer panel, preserving its
fractional origin. The edge band is derived only from that projected source.
Logical geometry stays in CSS pixels, normalized by the observed CDP layout/CSS
viewport ratio; the viewer DOM and viewport are never changed for comparison. Record physical viewer DPR,
normalization and exact source identity separately.

Six hosted cells are Wikipedia article/portal, GitHub, BBC, MDN and DuckDuckGo,
each at DPR1/2. Google remains a separate coordinator/Mac cell. Metrics alone
never establish user acceptance: retain and inspect the recording of normal
click/key/scroll flows, cookie banners, tab lifecycle and navigation. A blank
frame or stuck user is a FAIL. M0 does not close latency, strict Selkies parity,
provider, multi-hour stability, Vault or Path-1 gates.

Focused supplementary regression command:

```sh
node --test packages/kernel-client/browser-mirror-oracle.test.mjs
```

The Cloud shared harness entry and evidence protocol are documented in
`scripts/e2e-stack/README.md` in the paired repository. Do not relabel v1 source
receipts as v2 mirror failures or reuse acceptance results across source heads.
