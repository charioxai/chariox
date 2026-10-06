# MP-08/MP-10/MP-11 — phase17 IN PROGRESS

2026-10-06: assigned base f5d3b1da9 verified; md/display-perf. Protocol447/90
unchanged. Fresh clean base60Hz component53.07fps /1.99 owned cores. Capture-start
pacing56.50fps /2.05cores; independent row decoding58.28fps instrumented.
These do not close acceptance. scroll30 generates30unique updates/s; its prior
29.72fps is not evidence of a global30Hz cap. Real60Hz diagnostic added separately.

MP-08/MP-10 current commits: ed73a068c capture pacing;1a15d4a7c parallel row decode
+ owned profiling;d35a3c491 unbuffered helper profiler;3cf1da89f optional direct
libyuv I420 planes + cached portable conversion + normal four-credit pressure.
Fail-first parallel atomic failure test and direct YUV/credit tests pass after
fixes. Current SIMD component campaign in progress; honest15-row comparison next.
Python/V8 profiles and resource receipts retained externally.

## MP-08/MP-10 Coordinator asks

Supply paired Cloud kernel-browser display447 app integration and scripts/e2e-stack.
Short concrete interface spec: docs/MULTIDOMAIN_DISPLAY_PHASE17_PERF.md.
Existing Cloud f6cfd0066d75844dbab795cdf715789ec5fa37d6 is older Room display only.
Real-app red/green acceptance cannot run until that source/harness is supplied.

## MP-11 Review inbox mapping

Absolute agents/display/REVIEW_INBOX.md checked after each commit batch; latest
entry remains10:14 encoder licensing request. Previous #893 DPR/default and
encoder selection/provenance fixes retained. Narrowed scope applies to current
security anchors; no non-security blob backlog. Source protection, document and
visibility fences unchanged; atomic decoder abort closes every completed output.

## MP-10 Owner questions

Owner chooses x264 vs OpenH264 default later. GPU measurements need owner laptop.
No push/PR/CI/deploy/shared service change. Disposable profiles/public library
packages only; no provider/private credentials copied or printed.
