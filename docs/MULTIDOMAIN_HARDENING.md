# Multidomain multi-user and multi-client validation

This validation targets union local protocol 427 / relay peer 74, initially on
public union `32d4611a9`, then rerun on published stack-on-main `f3ad77def`.
That initial lane changes test fixtures and documentation only. Attributed refusal
hardening follows under local434/relay80; see [the refusal contract](MULTIDOMAIN_REFUSALS.md).

## Identities and access

A user is the kernel's authenticated `KernelCaller.user_id`; a local owner maps
to the configured kernel owner. An authenticated collaborator is a different
user, even when granted full membership in the same Room. Room membership does
not confer access to the owner's user-domain browser profile, App instances or
private notes. Multiple admitted clients of one owner share those surfaces but
retain their own input actor and prompt draft.

The focused local agent's runtime admission governs its user-domain MCP tools.
Browser and notes loaders are independent. A Room provider run without explicit
user-domain focus has neither loader. Changing focus revokes loaded tools and
in-flight admissions; refocusing does not revive an old admission or reload tools.
The native drill checks this across user browser, user App host, Room browser and
Room App surfaces. A focused browser observer can read an App projection but
cannot impersonate the human App frontend with browser input.

Relay client actors currently follow their authenticated admission identity.
Reconnecting with the same admitted client ID retains its takeover; another
admitted client cannot release it. Whether reconnect should create a new actor
lifetime is an owner decision. Relay protocol 74 has no client-disconnect event;
this validation does not introduce a transport authority or a new wire shape.
Local websocket actors do have connection lifetimes and disconnect cleanup.

## Reproducible checks

Focused Rust checks run the kernel test binary with one test thread and explicit
isolated fixture state. Include filters `kernel_browser`, `runtime::notes`,
`runtime::router::tests::notes`, `runtime::router::tests::user_app_views`,
`visible_region_capture`, `room_screenshot`, protocol-shape notes/App/screenshot
suites, and `transport::relay_peer::multidomain_union_tests`.

The opt-in `notes_user_and_room_native_integration_drill` additionally accepts
`CHARIOX_MD_HARDENING_DRILL=1`. Its existing normal-Unix-user / sandboxed Chromium /
explicit disposable `CHARIOX_HOME` requirements remain. The flag adds a full Room
collaborator, a user-domain App host and focus/access assertions. It writes
`MD-H-NATIVE-RECEIPT.json` next to the existing `MD-N5-RECEIPT.json`, outside source.
Native selection/layout are test CDP stimuli; observation and reanchoring remain
kernel isolated-world operations. Fixture frontend assets prove the host seam;
the separate Cloud live drill exercises signed App package admission and channel.

Cloud's `apps/web/scripts/multidomain-hardening-live-drill.mjs` drives the real
browser → local relay → kernel topology, two owner browser contexts, a separately
authenticated collaborator, local CLI transport under the disposable Unix owner,
remote CLI transport, and both full local and remote TUIs. The parent live harness
owns provisioning, process pinning, teardown and release acceptance limitations.
No hosted Cloud, real accounts, provider model prompts or runtime proxy is used.

For the initial base, set `MD_HARDENING_EXPECT_DPR2_RED=1`: the DPR2 pixel assertion
is an expected regression receipt. On the rebased/fixed stack, set it to `0` so
that the same failure is fatal. The probe asserts decoded PNG dimensions and
samples native pixels at the protected field's scaled coordinates. A failed
transport or invalid geometry is always fatal, never an expected pixel failure.
The published-stack rerun requires DPR2 green and preserves strict, structured
per-case authorization/stale-reference rejection checks.

## Published-stack result

On `f3ad77def`, the native Room/user browser/App drill, focused Rust and protocol
snapshots pass. The real relay/browser captures mask the protected field at both
DPR1 and DPR2. Both full TUIs open, list, project, call through keyboard actions,
capture into their own draft and close a shared-host App on the same kernel.

Nine live negatives remain unproven: foreign tab snapshot/screenshot, foreign
browser capture, foreign note-window observation, forged references, stale input
documents in browser and App, replayed capture correlation across identities, and
stale-generation capture. The kernel sanitizes these into a generic host failure;
that response cannot establish the expected authorization/stale-reference check.
These are failed validation cases, not demonstrated cross-user data disclosure.
Do not count arbitrary errors as successful security denials.

The evidence-only collection adapter delays the aggregate failure until all full
client positives finish. It preserves every individual failed assertion and a
nonzero final exit, without changing product code or refusal classification.
Exact commands, provenance, matrix and cleanup are recorded externally in
`/w/evidence/md-hardening/stack-main/`. The kernel still needs attributable safe
refusal information before these nine cases can satisfy release acceptance.

## Scope and limits

Every transport must pass positive access before its rejection tests count.
Cases cover forged references, other-user listings and calls, one-use selection
receipts, stale generations/documents, input takeover/release, browser and TUI
draft isolation, cross-origin overlay invisibility and DPR1/DPR2 masks.

`CaptureVisibleRegion.capture_id` is a correlation identifier, not a bearer token.
Reusing it cannot authorize another identity. Room screenshot artifacts are
protected by Room/attachment admission and current Vault observation epochs;
existing actual home/worker/relay checks exercise bounded transfers, wrong-Room
attachments, protected crop pixels, stale viewports and retired artifact receipts.
Their desktop pixel helper is deterministic, not a production X11 display claim.

The public input shape has atomic click/text/key/scroll operations, not key-down /
key-up held-state controls. The shared actor tests cover cancellation of running
controller input by human takeover and reject queued or disconnected actors.
No client-only held-key protocol was added. Notes and captures enter only the
initiating client's existing draft intake; no check sends a provider prompt.

## Attributed refusal follow-up

The md/refusals branch adds bounded kernel-issued reason codes and typed web/CLI
projections under reservation434/80. The nine generic-error cases now require
specific policy reasons, never arbitrary controller or transport failures.
The live harness records each accepted code/reason/retryability/client class in
the case receipt. Native agent/Room negatives require UserDomainRefused as well.
Unknown and foreign IDs share not_granted without an existence leak.
Validation is recorded in /w/evidence/md-refusals/REPORT.md FINAL.
