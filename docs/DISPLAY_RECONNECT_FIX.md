# MP-08/MP-10/MP-11: bounded display ingress scheduling

Release B (`b37f4504e4ce040a2d6c35dc56475315defbc861`) loses decoded
Web display with two Web viewers and local/remote TUIs. The fail-first
`red-b` run fails the initial concurrent decode window. A diagnostic worker
in `proxy-diagnostic3` records kernel display ingress `full=true`, followed
by proxy client closure and forwarder `lease_closed`. The private Selkies
adapter is downstream of this closure.

The relay reader can process a buffered burst without yielding. Its bounded
16-event channel fills before a ready consumer runs; `try_send` then closes
the healthy stream as though its consumer were stalled. The relay viewer
writer has the same scheduling seam. Both paths now yield after accepting
an event, outside state locks, before processing the next buffered packet.
Queue bounds, order, encryption and fail-fast closure for stalled consumers
remain unchanged. No serialized protocol shape changes; local protocol 370
and relay peer protocol 64 remain unchanged. Fixed diagnostic reasons never
include private adapter output or packet contents.

## MP-08/MP-10: reproduction and focused checks

`healthy_display_ingress_drains_bursts_without_losing_fragments` submits 64
events on a current-thread Tokio runtime with a ready consumer and checks
exact ordered delivery. It fails without the scheduling handoff. The stalled
consumer case must still close its own stream while another stream accepts
traffic. The relay regression exercises the real relay connection path with
the same buffered burst.

Run the repository tests with the configured Rust toolchain:

```sh
CARGO_BUILD_JOBS=6 cargo test -p chariox-kernel healthy_display_ingress
CARGO_BUILD_JOBS=6 cargo test -p chariox-kernel stalled_display_ingress
CARGO_BUILD_JOBS=6 cargo test -p chariox-relay healthy_display_viewer_drains_bursts
CARGO_BUILD_JOBS=6 cargo test -p chariox-relay
node --test apps/cli/scripts/lib/room-repetition-gates.test.mjs
```

The lane's full-kernel Cargo invocation waited on the shared build target
and was cancelled before execution. Its focused kernel proof compiled the
actual new ingress module and verbatim state methods with unrelated state
scaffolding omitted: RED 1 pass/1 fail, then GREEN 2/2. This is a focused
source proof, not a full-kernel-suite result. The actual relay crate's
buffered-burst test failed first, then its display suite passed 20/20 and
its complete suite passed 96/96, with no skips. Harness comparisons passed
7/7. The separate crypto/forwarder source checks passed 10 tests with two
Docker integration tests explicitly unexecuted.

## MP-08/MP-10/MP-11: live replay and provenance

The imported `live-room-repetition-drill.mjs` harness is documented in
`ROOM_REPETITION_GATES.md`. `LOOPS_PHASE=reconnect` selects 50 reconnects
with two Web viewers and repeated real local/remote TUI reattachments. Each
cycle requires both decoded canvases to survive a two-second window and
their active-browser pixels to advance, while Room, environment, generation,
tabs, agents, history and action identities remain stable.

Worker runtime source is `e9a877452a78a4b8385da3937bc61c647dd92464`;
the running harness source is `addb331445b97a324198e6d9f9452d3d7f927c29`.
The worker candidate is unsigned and built inside B's Linux base. Its live
worker binaries are checked before viewers attach:

| Binary | SHA-256 |
| --- | --- |
| kernel | `7b8e85fb6509e453aca25c0bb60ce883efc4cd30d4d66f474f6ddff6bc703600` |
| relay | `1b40d7bae62cb02605074a9471cf200c3ebda5f4a6216473d8720af809905535` |

The headed candidate image is
`sha256:c27c37e3bc1d262972ca263a10f1bd48d964196e00faebc9eb0ddeaed8854589`.
Its runtime source digest is
`1e4788417859c70759cd62e2ab75c22c975d35642a2a1efa529e0c22aee3410f`.
Host kernel, host relay and client remain B; the retained Web build remains
Cloud `94fda0ec7af67ce37c3da19e0dd6588c1131ae8e`. These identities must
not be relabelled as signed release C. The live replay exercises the worker
kernel handoff through B's host relay; the relay handoff is covered by its
actual crate tests and awaits integrated-C runtime replay.

The live result and cleanup receipt are retained under
`/root/.codex/evidence/browser-resume-20260930/streamfix/green50/`.
The run completed GREEN on 2026-10-01: 50/50 reconnects, both viewers
decoding and advancing, no gate failures, no ingress-full events, and empty
owned-resource cleanup findings. Starting/Running OCR discovery passed;
after session termination the same run was Ended with an empty tools list.
Lane images, temporary binary copies and runtime scratch were removed after
their identities and source receipts were retained. Shared images, build
caches, credential profiles and other lanes were left untouched.
The replay does not exercise real provider turns, hosted infrastructure,
fresh-machine setup, crash reconciliation or the full Drill H workload.
Signed-C aggregate replay and independent review remain required. No MP item
closes from this lane's source tests or unsigned candidate run.

## MP-08/MP-10/MP-11: OCR discovery

Both `slice_ocr` and `chariox.slice_ocr` are advertised for active Starting
and Running Room-bound runs. Ended runs intentionally return an empty tool
list: `get_runs_by_runtime_mcp_auth_token` excludes Ended runs, and tool
discovery requires that active binding. Retained binding metadata alone does
not establish an active invocation context.

Loops' default-model lifetime probe and the lane's active/ended discovery
checks support that contract. Bench2's original missing-OCR report did not
retain lifecycle state or the failed tools list; its exact cause remains
unproven. Its later active idle-run probe advertises OCR. There is no
established active-run OCR defect and no OCR runtime change in this fix.
