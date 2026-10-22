# Session selection and offline kernels (MP-08 / MP-10)

These scoped acceptance rows cover the owner's 2026-10-16 session reports.
They apply equally to ordinary and managed kernel placements. They do not
close the broader MP-10 parity or Browser/Computer acceptance matrix.

Use the real built web entry, TUI, kernel and hosted WSS relay, normal product
enrollment and a product-linked provider profile. Use an actual working Room
with provider history, and a second Room with a different agent identity.
Pin every source head and binary hash. Record the real clicks/keys, screen
video, screenshots, public connection diagnostics and cleanup. Supplementary
source regressions must fail on the assigned main baseline before passing on
the fix; they cannot replace these live rows.

## Required rows

| Acceptance ID | Required real flow and result |
| --- | --- |
| `apps-tui-ux/OWNER-24` | Open A, then choose B through the normal session/agent controls, both on one kernel and across kernels. Delay B's responses, including an offline B. The selected canvas changes immediately to B's own cached snapshot or placeholder. No painted frame after selection contains A's panes, transcript or draft. A late A request must not restore A over B. Exercise the built TUI's session switch as well. |
| `apps-tui-ux/OWNER-25` | Stop only a disposable kernel's network or SIGSTOP its exact owned PID, wait beyond heartbeat freshness, then open its Room. Show its normal canvas and pane footer with an offline kernel pill; never a grey blocked canvas or a ready footer. Retain any known history only for that Room. A short restart stays reconnecting; the shared footer becomes offline after 60 seconds away. Resume the same kernel and recover through normal controls without losing Room identity or starting a duplicate provider run. Check the real TUI's shared connection chrome during the same fault. |
| `apps-tui-ux/OWNER-16` | While that kernel is offline, inspect the account control with and without cached account metadata. Show offline, preserve its last-known aliases when available, and prevent changes or launches. Never infer “no accounts” from a missing response or borrow another kernel's accounts. Reconnect and recover the actual account inventory. |

Run the web flows at 1440×900 DPR1 and DPR2 through approximately 8 Mbit/s
client uplink and at least 60 ms measured round-trip latency. Watch every
recording before assigning PASS; a stuck, confusing, blank or grey view fails
the row, with the timestamp retained. A source-only or unreviewed recording is
NOT RUN for live acceptance. Name any missing real resource and its owner.

## Authority and cleanup

Cloud target admission must reject expired kernel heartbeats. A relay socket
being open and a cached inventory existing do not establish kernel readiness.
Fresh authenticated runtime traffic may keep a live Room visible during a
directory outage. Connection status and account display follow that Room's
kernel. Cached display never creates an attachment or a second session authority.

Resume paused processes before stopping them. Stop only recorded owned PIDs
greater than one, remove disposable kernel identities and receiving provider
profiles, and stop the lane's own hosted stacks. Keep evidence outside source
trees. Do not touch shared profiles, reviewer services or other lanes.
