# MP-08 / MP-10 / MP-11 — display region 519/127 drill

Supplementary regressions: `node --test
apps/kernel/slice-linux-docker/docker/kernel-browser-display-regions.test.mjs
apps/kernel/slice-linux-docker/docker/kernel-browser-window-region.test.mjs`;
run the kernel protocol-shape, admission, protected metadata and embedded asset
tests through the builder's cargo slot. These checks do not close acceptance.

Real acceptance uses the integrated kernel/relay/Cloud heads with mirror v2,
the concrete ChromeWindows/arbiter, and the normal web entry. Start from fresh
state with an explicit lane-owned CHARIOX_HOME, record binary hashes and first
home log line, and run the base as the separate RED comparison. Regions must
use public display_subscribe, CXD1 delivery, the shared presenter and ordinary
ACK pump. No viewer CDP shortcuts, fixture dismissal or DOM injection.

At DPR1 1920×1080 and 1440×900 plus DPR2, visit the Wikipedia article and
portal, GitHub repository, DuckDuckGo search, BBC and MDN through normal native
omnibox clicks and keys. Dismiss cookie prompts normally; exercise new/close
tab, popup, navigation, typing and scroll. Assert mirror wire2, then choose
kernel-issued tile references. Verify video inside each opaque region and
window-lifetime video in the native band; dropdowns require admitted auxiliary
regions. Retire a crop on resize, changed tile geometry/identity, protection,
navigation and browser restart. Old input and late frames must be refused;
new subscriptions paint without selecting a viewer rectangle.

Native band (528): open the web Browser on a kernel with display on; the band
must paint from the state's `chrome_window` without a viewer-chosen window. Switch
Tabs in Chrome's own strip and confirm the page follows `active_tab_id`; start a
kernel with display off and confirm `chrome_window` is null and the web shows its
own strip and address bar. On a host without the AppArmor profile the state's
`browser_lockdown` is `unavailable` and the session bar says so.

Band gestures and popups (530): with the mouse only, middle-click a Tab in the band
(it closes), right-click a Tab (Chrome's menu paints over the band where Chrome shows
it; pick "New tab to the right"), drag a Tab along the strip (order changes; dragging
below the strip never tears it off), open the ⋮ menu and the extensions (puzzle) menu,
type in the omnibox (the suggestions dropdown paints and an item can be clicked), and
in chrome://extensions remove then disable "Chariox tab groups" (the next band input
reloads it; until then band input is refused). Repeat at DPR1 and DPR2.

Use the hosted WSS path, shaped ~8 Mbit/s and measured RTT ≥60 ms. Strict
paired Selkies reruns use the same integrated head on the coordinator's quiet
host (HOST_REQUEST), with matching geometry. Miguel's 2026-10-16 decision makes
keystroke-to-paint PASS require Chariox P95 ≤ Selkies P95 in that same paired
run. Keep measured wire RTT and RTT+60ms beside every keystroke result as
tracked values. Other latency metrics retain P95 ≤ measured RTT+60ms and their
existing comparator requirements. Require FPS ≥Selkies and ≥30 where streamed,
and ≤one CPU core. Run a two-hour DPR2
region session and reconnect. Watch all recordings; report a stuck, confused
or blank/white user flow as FAIL with its timestamp. Google is N/A-AUTOMATED.

Evidence belongs outside Git under the lane's evidence directory. Retain
commands/exits, source and binary identities, screenshots/video, numeric
transport/timing and resource samples. Stop exact owned PIDs and stacks, remove
only owned scratch and runtime identities, and write the acceptance update.
Missing Chrome owner seams or a quiet host are named coordinator dependencies;
a synthetic lease never substitutes for a live acceptance row.


## MP-08 / MP-10 / MP-11 page placement 529/137

First run the focused supplementary version/snapshot regression:
`node --test scripts/browser-display-placement-529.test.mjs`, the Rust
`mp08_mp10_mp11_browser_display_placement_529_snapshot` through cargo-slot,
and the Cloud page-runtime-regions, display and browser-mirror2-regions tests.
The old528 source must fail the version/floor assertions. These are source
regressions, not real-path acceptance.

On the refreshed integrated pair, repeat fresh normal Browser startup at
1440×900 and1920×1080 DPR1 and DPR2 with the real built web entry, kernel and
hosted WSS relay. Record protocol529 or newer, the first wire2 paint and the
native window geometry. From the public-site list above find a genuine opaque
canvas/video/cross-origin slot, including one partially clipped by scrolling
or an ancestor frame. Navigate/scroll with normal keys and clicks; do not
inject a canvas, set slot coordinates or use viewer CDP to choose a crop.
Capture the ordinary subscribe reply and verify `region_offset` is present,
finite and nonnegative. Watch the recording: video must align with the still
and surrounding page, retain the original layout at both densities, and
accept a normal click at its visible location through the same kernel lease.

Scroll/resize and navigate to retire that lease; late frames and input must
not paint or act at its old location. A fresh lease must return new placement.
Capture a partially offscreen or masked case and verify it stays within the
owned page/window and protection fences. Kernel refusal stays a still with a
recorded reason, not an invented placement. Confirm a528 kernel keeps stills
without page Region requests while native band519 remains usable. The old
producer without placement is RED for page video, not accepted as a fallback.

Report startup/placement/input/retirement separately with exact heads,
screenshots, recording review and first failing seam. Only after functional
readiness passes request the quiet host for strict paired Selkies timing and
the two-hour soak; b3 source/functional checks cannot establish these gates.

## MP-08 / MP-10 / MP-11 timed ACK lane (536/144)

Before changing ACK scheduling, preserve the red held-reply regression and
90ms RTT/30fps cadence result from `apps/browser-display/ack-pipeline.test.mjs`.
The cadence target is ACK-age P95 <=105ms. Preserve Rust red/green receiver-hold,
adaptive allowance, generation refusal and stalled-client recovery tests;
round-trip the extended536 snapshot/hash through cargo-slot. These are
supplementary checks only.

On the integrated real kernel/relay/web path, use normal play/pause and scroll
on MDN's public flower video and the Commons video at DPR2, with three two-second
steady windows and a post-correction physical-pixel FPS sampler. Keep the full
kernel log and numeric `push_oldest_unacked` samples; count `push_gate_closed`
transitions separately from closed polling, and record `push_ack_rtt` beside
raw `push_frame_ack` age. Absent transition telemetry means unknown counts.
Collect `chariox:browser-display-timing` (and Room's existing timing event)
from the real viewer: `client_receive` starts at raw socket receipt; the track
`committedAt` and `displayPresentedSequence` require an exact decoded-media-time
rVFC join, while `writtenAt` only means track writer completion. This compositor
clock still requires comparison with recorded physical pixels.

Repeat normal input while ACK replies are delayed, stop ACKs to prove the
kernel gate closes, then recover without an unbounded request queue. Check a
kernel without `ack_timing`: the viewer sends no `held_ms` and the original
stream still works. Use the hosted shaped path and the same quiet-host Selkies
control; apply the keystroke parity gate and track RTT+60ms, with other latency,
FPS and CPU gates as above. Keep current source identities
on every receipt and watch the recordings before grading.
