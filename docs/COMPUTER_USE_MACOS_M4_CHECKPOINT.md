# M4 native targets and protection source checkpoint

R10 starts `cu/macos-m4` exactly from published M3
`7d8468b309ee2f296a0173a1365fde6b36ca23cf`. This is a disabled signed-fixture
source checkpoint, not native or hosted acceptance. The retained r6 helper has
not been rebuilt, signed or launched. Batch this source into the next combined
build rather than replacing the helper that awaits the morning grant.

`TargetRegistry` walks at most 128 nodes and 32 levels with a two-second deadline.
Secure classification runs before metadata or children. Failed classification
withholds the observation. No AX value, selection or description is read.
Editable controls expose no title. Labels are scrubbed before size reduction.
Refs are random, transient and tied to process launch identity, observer,
helper generation, surface geometry, protection revision and observation revision.
A new observation, failed validation, observer notification or two-second expiry
invalidates refs. Invalidation during a walk cannot commit a fresh observation.

`NativeTargets` binds one selected AX window and installs a main-run-loop
AXObserver for destruction, movement, resize, value and focus changes. Supported
notifications supplement mandatory live validation. Action use checks the stored
metadata and all ancestors' secure classification, permissions, focus, ancestry,
hit testing and WindowServer visibility. Only enabled AX buttons advertising
AXPress project the existing `click` action. Refs are consumed before dispatch,
including when post-effect verification fails. There is no raw AX handle in the
response and no arbitrary AX action execution.

The helper now implements the existing shared `snapshot`, `ocr` and
`target_action` commands. Snapshot retains `surface_id`, `generation`,
`tree_revision`, `nodes`, `complete` and `fallback`. Nodes retain `target_id`,
`role`, `name`, `states`, `bounds` and `actions`. Bounds convert global points to
top-left surface pixels, including negative origins and 2x scale. Projection
uses the shared 3072-byte node budget; incomplete results request OCR fallback.
One current observation per paired seat is intentional: another observer's
snapshot retires earlier refs rather than transferring their authority.

Local Vision accepts only immutable protected RGBA frames returned by the paired
capture pipeline. It never receives the raw SCK image. Recognition is bounded
to four million pixels, 128 results and 8192 scrubbed text bytes. It uses accurate
English recognition with language correction off. Normalized bottom-left boxes
convert to global top-left points; confidence, alternate candidates and overlap
mark ambiguity privately. Public OCR reuses the existing text result and refuses
ambiguous recognition. Query lookup remains unsupported. Authority, permission,
focus, geometry and protection are checked before publication. This source does
not claim universal OCR languages, latency or a useful AX-absent fallback: capture
still requires proven secure-field coverage and therefore withholds opaque AX.

Miguel's private masking scope is only Vault-filled plain text fields. No app
category or ordinary-text heuristic adds private masks. Existing secure-field
protection remains. The existing kernel registry combines active and retired
Vault values with browser/X11 refs. Those refs cannot prove a Mac field's bounds.
`NativePrivacyPolicy` validates that registry and scrubs retired values, but
withholds pixels and AX/OCR while mapping or provenance is unresolved. This
replaces the previous blanket native-window mask for nonempty registry policy.
Empty/invalid region geometry now refuses before encoding or OCR.

The coordinator must establish authenticated Mac plain-text field mapping and
private UI exclusion provenance before field-scoped masking can be accepted.
If that requires a shared serialized change, request its assigned version first.
No M3 wire change was needed here. Local 497 / peer 106, frozen display #950
`f04b4006b3b8b1b833560022128b0345db873e5a`, existing events/CXD1/crypto and client
minima remain fixed. 499 / 108 were not allocated. Cloud#325 remains unchanged.
Mac provider Computer tools remain disabled by their existing Linux-only gate.

Latest reviews #920 `5da701e0c` and #923 `edfee0811` found the two caret runners
still bypassed the pointer lease. Both now bind PID/window/bounds and use the
native single-warp fence for movement and cleanup. Every sampled takeover retires
the lease, even if focus returns before cleanup. The fix is mirrored in M0-M3.
#957 `50cd5fe15` has no additional actionable findings. No new P1 was reported.

Evidence is under
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r10-20261011`.
Behavioral red/green logs cover empty private geometry, paired target/OCR policy
dispatch and both real runner boundaries with their native scripts in a Node VM.
Missing AX/OCR/policy API compile logs are supplementary. Removing fresh target
validation, frame-stamp checking or Vault withholding fails the focused tests.
Real Vision read a generated public canary and could not read its opaque protected
copy; these images stayed in memory. This is component evidence only.

Focused Swift checks and the arm64 macOS 14 helper source typecheck use `-j2`
with one Swift build at a time. The external runner stops at critical memory
pressure or three GiB owned process-tree RSS. Python and Node runner regressions
also pass. No whole-kernel tests ran on this Mac; those belong on b3. No native
AX observation/action, OS capture, permission request, provider, hosted viewer,
new grant, signing key or installed bundle was used.

M4 remains BLOCKED on field-scoped Vault mapping, private UI exclusions, useful
opaque-AX fallback, query/language coverage, native no-leak canary and stale-ref
drills, matching build/grant and the real Web/local-TUI/remote-TUI/provider matrix.
M2/M3 remain disabled until those gates pass. Owner windows still refuse. Rollback
keeps the helper disabled and removes the scoped drill configuration; it never
modifies the granted bundle or attempts to release unknown OS-owned input.

Native API references: [Apple AXUIElement](https://developer.apple.com/documentation/applicationservices/axuielement)
and [Apple Vision recognition](https://developer.apple.com/documentation/vision/vnrecognizetextrequest).
