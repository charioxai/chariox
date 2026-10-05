# MD-DISPLAY-02/04 — Phase 5 ready for coordinator review

Branch agent/display-impl; execution/kernel build933d6222d. Protocol419,
relay wire unchanged, flag off. No publishing action. Source/binary fingerprints,
commands/exits, embedded assets and receipts: external phase5/provenance.json.
Doc:docs/MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md. No MD/MP acceptance closure.

MD-DISPLAY-02 final-v3: seven cases exit0; all140 input counter acknowledgements,
exact settled/final RGB, same-stream navigation, stale-document rejection,
takeover/focused-MCP fencing/owner input/release and cleanup pass. Local
p50/p95 68.5/81.6ms; WAN40/80/150 169.6/432.5,287.5/310.9,469.0/498.1ms.
MTU1500, offloads off, 1%loss, jitter and5/2/1Mbps caps affect only viewer/proxy
TCP inside owned namespaces. Initial exact repairs5.35/8.39/13.17/18.21s.
Relocated-static-100s-v3:255 polls,20/20 clicks,69.6/84.3ms, navigation/exactness
and cleanup pass while a process-local hook blocks shared-checkout TypeScript.
Both source933d6222d, clean trees, copied binary SHA bound individually.

MD-DISPLAY-02 media: canvas/video1.18/1.19 drawn fps and0.81/0.76 frame-event
Mbps; settled exact in1.95/1.90s. Dense scroll0.25fps,1.44Mbps,7.73s exact
settle and198/232ms post-scroll clicks. Diagnostic capture RPCs add unpaced
bytes; live PSNR includes temporal drift. No matched moving-codec/Selkies/WAN
baseline claim. The doc's short Selkies comparison preserves old geometry/source.
Moving/WAN quality remains weak; feature is reviewable, not ready for rollout.

MD-DISPLAY-04 changes: stable-only refinement; bounded lossless tile batches
sized to negotiated budget; one credit bounds source/encode/transport queues;
full protected capture for scrolled/zoomed/unknown origins. Native VideoToolbox/
ScreenCaptureKit and Media Foundation/Graphics Capture integration plan plus
unavailable unregistered stubs; no native execution claim.

MD-DISPLAY-04 review mapping:

- Inbox03:42 navigation closure:7197ac202 recreates capture on loader change.
  navigation-fail-first-v2.log fails only after navigation (v1 wrong-target setup
  retained);46-test fixed suite and every final live case pass. Same display
  subscription receives independent video on new document; old input rejected.
- Inbox03:42 hard-coded TypeScript/startup leak:7197ac202 resolves supplied public
  tools under finally. dependency-fail-first.log + exact empty-root cleanup;
  dependency-fixed-start reaches deliberate missing binary and cleans; real
  relocated100s final replay confirms no shared-checkout dependency.
- Prior inbox02:17 registration/cursor/uncached-credit replay remain fixed by
  c8518a3e2/69e1782ef, with original phase4 fail-first receipts preserved. Final
  static100s and focused13 Rust checks include registration/protocol/queue/actors.
- New oversized/moving repair:2cbfe5618, repair-fail-first-tests.log -> passing
  bounded reconstruction/motion invalidation tests. Early live harness still
  compared one partial batch; seven RED final/ receipts retained.29d3698e4 waits
  for unchanged before checking exactness (new bounded drain test).
- New scroll source1/viewer0:933d6222d full protected capture at unsafe origins;
  scrolled-crop-fail-first.log ->46 passing Node checks, final-v3 scroll20/20
  probes with exact pixels. Previous final-v2 scroll remains RED, exit1.

MD-DISPLAY-04 checks:46 Node tests,13 focused reserved-slot Rust tests, slot2
build pass (40 existing warnings), diff whitespace clean. Source shape/hash guards
remain419. Interruption-v2 exits130, settles owned runtime, removes namespace.
Earlier invalid interruption copied an in-progress linker output and failed
before signaling; retained RED.79ad69619 adds ELF validation and copied SHA.

MD-DISPLAY-02 cleanup: every completed final namespace and exact-root process
inventory empty; disposable state removed. Own replay worktree removed;5.91GiB
own compiler incremental output removed after exact compiler-file inventory.
Current public kernel binary/public dependencies retained for coordinator replay.
No shared caches/services/containers/images/credentials/keys/reviewer state touched.
All screenshots/diffs/JSON/logs external under
/root/.codex/evidence/browser-resume-20260930/display/phase5/.

MD-DISPLAY-04 next gates: owner budgets/design, continuously credited WAN client,
protected persistent source/encoder, coordinator Cloud wiring, native/live Vault,
slow viewer/reconnect, cursor/IME/file chooser, multi-viewer and Room migration.
Research agent/display branch untouched. No GitHub CI, push, PR, merge or deploy.
