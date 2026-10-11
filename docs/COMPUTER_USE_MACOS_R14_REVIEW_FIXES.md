# R14 review fixes, component evidence only

The newest #962 review at `5c2ee2eb22b18595bf51721daf8020ca8667354b`
contains two P2 findings. The request said "all three"; no third finding was
supplied. #963 at `20002214e` says no actionable findings; the newest #964
file at `4dd84b5ab` contains only its review heading. There is no third source
fix invented from these files.

Physical activity now uses a kernel-only Desktop takeover transition with a
fixed physical-owner actor. It supersedes terminal ownership and pending
terminal takeover under the actor lock, cancels active terminal or agent work,
and retains reservations until cancellation finishes. Ordinary terminal
requests still refuse another human owner's lease. The paired helper callback
uses this transition; explicit authenticated release still waits for successful
helper reset before releasing the physical lease.

The fail-first adapter regression returned `desktop takeover denied` for a
terminal-owned move. It now passes terminal and agent move/click cancellation,
authority retirement, and fencing until completion. Additional tests cover
idle takeover after terminal takeover or completed input, stale generation,
ordinary terminal refusal, same-pairing reset/release/resumed agent input,
and replacement of pending terminal takeover. The production authenticated
release integration test is extended and compile-checked, not executed here.

The input drill requires two distinct click destinations and expected selection
values, and reads selection independently before each dispatch. Initial input
cannot pass with an already correct selection. Resumed input cannot pass by
acknowledging without changing the fixture. Both negative tests passed before
the fix when they should have rejected, then correctly rejected after the fix.
Missing/reused resume targets and no-effect initial dispatch also reject.

Evidence is `/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r14-20261011`.
`takeover-red.log` and `resume-red.log` preserve behavioral failures.
`adapter-final.log` records 39 focused real actor/ledger/adapter/release checks;
`node-final.log` records 28 Node checks with zero skips. These are component
checks. The harness imports production ledger, actor and release modules,
uses production cancellation source and the real Mac helper adapter, and
substitutes minimal outer host plumbing and an authenticated fake helper.
`kernel-check.log` records successful guarded `cargo check --tests --offline
-p chariox-kernel -j2` on the production kernel and test sources. Existing
warnings remain. Whole-kernel test execution belongs on b3 and is not run here.

No serialized protocol shape changes. Local 497, relay peer 106, frozen #950,
Cloud #325 and crypto contracts remain fixed. M2-M5 remain BLOCKED on matching
build/grant and real native/client/provider proofs, with the existing M4
protection/query/language and M5 codec/performance/signing prerequisites.
The retained r6 helper is unchanged. No production helper build, signing,
replacement or launch; no Swift compile, TCC, sudo, Keychain, credentials,
provider/hosted run, push or CI occurred.
