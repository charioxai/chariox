# R6 final helper waits for one re-grant

All r6 helper changes entered one rebuild from M2 source checkpoint
`5f6bac03c8ccfda6ae28fe803ad58e9185da5b97`. It includes the M0 foreign-window
fence, the M1 kernel-channel revocation path and the first M2 protected capture
source. The previous granted helper and Recheck bundle were not overwritten.

The only owner step is to re-grant this exact build bundle in Accessibility and
Screen Recording through System Settings:

```text
/Users/miguel/.chariox/dev/cumac/build/r6/Chariox Computer Helper.app
Identifier: ai.chariox.computer-helper
CDHash: 8b87ab7229c79a45f21eaddb7b2ef833095cf9d0
Executable SHA-256: b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028
```

Its LaunchServices identity receipt has parent PID 1 and a valid ad-hoc signature.
The nonprompting preflight returned `refused: permission (accessibility-denied)`.
An `open -W` exit warning also occurred after the helper had produced its full
receipt. No input was attempted and no permission request API ran. This is
BLOCKED pending the re-grant, not a live fence or revocation PASS. No document
opening, owner pointer placement or fixture clicking is required from Miguel.
Do not rebuild/re-sign this bundle after its grant.

After the re-grant, the agent/coordinator runs from
`/Users/miguel/chariox-worktrees/cumac-m2`:

```bash
export CUMAC_BUILD_DIR=/Users/miguel/.chariox/dev/cumac/build/r6
export PYTHONDONTWRITEBYTECODE=1
python3 apps/macos-computer-use/text-click-drill.py --suite
python3 apps/macos-computer-use/textedit-click-drill.py
python3 apps/macos-computer-use/fence-drill.py
```

The cover runner independently checks a foreign nonactivating WindowServer panel
above the fixture, exact-point coverage and unchanged frontmost PID/selection.
Both explicit and pointer clicks must return `foreign-window-cover`; stale
pointer refusal is checked separately. The helper samples fresh CG z-order at
every mutation fence, including AX focus/selection/press/scroll and bounded
per-PID typing. A missing/stale target or unproven window stack also refuses.
All runner-owned windows must close. The scripts honor the lane GUI_HOLD rule.
Grant-dependent drills must wait for the grant even if the GUI_HOLD timeout ends.

The kernel seat must use the matching separately built binary. Its retained
location and hash are in the lane's GUI_READY.md and runtime-manifest evidence.
Its public CDHash is `518b38b5cad455ee47274402ad86d0c20eda6d06`; the helper's sealed
private drill allowlist admits that exact kernel. The kernel admits this exact
helper CDHash through `CHARIOX_MACOS_COMPUTER_HELPER_DRILL_CDHASH`.
The kernel head is `5ff2f8b862a57b9c0bc2428e080ad93f222e3d7e`.

For M0-REVOCATION, the coordinator prepares a separate real kernel with an
explicit fixture scope in `CHARIOX_MACOS_COMPUTER_DRILL_SCOPE`. The JSON object
has exactly `pid`, `window`, `owner_window: false`, and `target: "text"` for a
live build fixture opened with `--unattended --text-drill`. Do not use the
identity-only seat for acceptance. Use an already approved durable kernel home;
verify retained public pins and the tested protected backup before any identity
use. Never initialize or delete credential-bearing state in task scratch.
The re-grant is the only owner action; the coordinator performs kernel setup.

Create a public scope file outside source containing those fields plus `bounds`
with Quartz `X`, `Y`, `Width`, `Height`, `scale`, the fixture's `point` with `x`
and `y`, and `expected_location`. Use the fixture's independently checked probe
at UTF-16 index 126 and the current WindowServer geometry. The kernel bootstrap
receives only the four admitted scope fields, not the extra probe metadata.
Pass the existing private local-auth token file without printing its contents:

```bash
node apps/macos-computer-use/seat-revocation-drill.mjs \
  --port PORT --token-file /absolute/protected/local-token \
  --scope-file /absolute/public-scope.json --fixture-log /absolute/fixture.out \
  --evidence /absolute/evidence-directory
```

The runner proves a live paired caret mutation and protected paired PNG, then
calls kernel `revoke_grants` mid-session, requires a bounded Stop acknowledgment,
refuses the old seat's next input and checks selection remains unchanged. It
never launches a kernel, creates keys, alters TCC or deletes runtime state.
The signed fixture exits through its watchdog; close only that owned process if
cleanup is needed. The live in-flight/fatal-held-input matrix remains M3 work.

M2 shared-viewer delivery is still BLOCKED. See
`docs/COMPUTER_USE_MACOS_M2_CHECKPOINT.md` for the native attachment contract,
credits, damage tiles, exact repair and encrypted relay integration still needed,
plus M3-M5 dependencies. Re-grant alone does not establish those missing pieces.

Preserved older bundles retain CDHashes `d171e9751ee725169cdb777d6ffc3c46e6bdd86b`
and `425753b7ce44dd772d2e428f3bc36b634d69eaeb`. R5's foreign-cover failure remains
historical live evidence until the new helper passes that exact drill. R4's
independent TextEdit reference and historical index-84 cause remain unresolved.
Evidence is outside Git under
`~/.codex/evidence/browser-computer-use-macos/cumac-r6-20261010/`.

## R7 review fixes are source-only

Reviews of #920 @5973ba185, #923 @43fd9f1a6 and #957 @777b256c4 arrived in r7.
The source now refuses same-process covers, verifies the intended build launch
receipt, latches failed owned release through the kernel as owner-reset-required,
retires a permanently invalid geometry binding, fails capture on AX secure-field
classification errors and fails the fence verdict on unclosed owned windows.
These helper/kernel fixes are NOT in the retained r6 binaries. Do not rebuild
those binaries while the pending grant waits. The exact r6 suite above is
partial historical-failure replay, not live validation of these new review fixes.
Batch all helper and matching kernel changes into the next rebuild after the
display allocation. A failed owned-release latch survives Stop/Start/reset;
the owner must physically release input and explicitly restart the kernel.
Kernel restart is not proof that a physical release succeeded. Never silently
retry the failed Action. Fatal crash/release recovery remains an M3 gate.

M1's source only supplies identity/scoped bounded input. The paired protected
capture/replay steps require the separately published
[draft #957 at 777b256c4](https://github.com/charioxai/chariox/tree/777b256c41875202ad0921d33aac5c40541e946a/apps/macos-computer-use)
and the coordinator's revised local M2 suite. That requirement does not imply
that M1 implements protected capture or that the unpublished r7 source was
already rebuilt or granted.
