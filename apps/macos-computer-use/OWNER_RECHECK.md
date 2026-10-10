# R9 owner grant and unattended M0-M3 suite

The retained r6 helper remains unchanged while the morning re-grant waits.
Its executable SHA-256 remains
`b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028`.
Do not rebuild, sign or replace that bundle. Its historical replay below cannot
validate r8/r9 source, protocol 497, physical-human fencing or fatal input reset.

The full source suite requires one grant on one next combined, matching build.
Batch all current M0/M1/M2/M3 helper changes into that coordinator-approved build
under `/Users/miguel/.chariox/dev/cumac/build`. Publish its exact public helper/
kernel SHA-256 and CDHash pins before the owner grants it. Do not rebuild it after
the grant. This is a future gated recipe, not an owner-staging or acceptance PASS.
No new matching bundle was built, signed or launched in r9.

After that single grant, the runner does all fixture setup and cleanup. It runs:

1. M0 exact-build caret/TextEdit and foreign/same-process cover fences, including
   pointer takeover and permanently retired restoration leases.
2. M1 paired identity/lease/Stop/revoke and strict in-flight cancellation. Include
   oversized text, unsupported key and pre-dispatch permission refusals followed
   by usable input. Missing release confirmation must still refuse Stop/Start.
3. M2 protected paired capture, independent PNG/damage/repair checks and the real
   hosted encrypted Web/local-TUI/remote-TUI native viewer gate described below.
4. M3 experimental fixture input with `CHARIOX_MACOS_COMPUTER_INPUT=1`, observer
   health, physical-human actor projection, denied agent input with unchanged
   independent selection, explicit owner release and resumed input. Run fatal
   owned-press death last and require owner-reset-required plus Stop/Start denial.

Use `prepare-seat.py --input-drill` for the M3 public fixture and
`native-input-drill.mjs:runNativeInputDrill` with the coordinator's actual admitted
agent/owner transports and independent native event/fixture receipts. It creates
no identity, credentials or endpoint. The coordinator must supply the verified
fatal press hook that observes public-fixture key-down while the bounded 100 ms
Action is pending and kills only the exact paired helper. Preserve the failure
receipt on any failed check or cleanup. A mocked hook is component evidence only.
No implementation of that real transport/fatal-hook orchestration was executed
here; this dependency remains BLOCKED and is not silently replaced by a mock.

The observer performs nonprompting listen-event preflight. If Input Monitoring
is separately needed, report the missing opt-in and stop the M3 gate. No agent
requests TCC, changes Keychain or retries mutation to turn a failure into PASS.
Fatal death intentionally leaves an unresolved OS press if cleanup cannot be
proved. No new Action may follow; owner physical release and explicit restart
are required before reuse. Restart alone does not prove the press was released.

Whole-kernel test execution belongs on the coordinator's b3 lane. This Mac
ran focused checks and guarded kernel test-source compilation only. No b3 run,
hosted transport, live tap, GUI input/capture, real provider or new grant is claimed.
See `docs/COMPUTER_USE_MACOS_M3_CHECKPOINT.md` for source scope and remaining gates.

The old pinned-binary instructions below are retained as historical replay.
They are not the full M0-M3 recipe and do not authorize replacing its pinned build.

# R8 owner recheck and unattended acceptance

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

After the pending re-grant, run only the standalone r6 block below. It is
historical failure replay. R8 source fixes and native encrypted delivery need
one later combined helper/kernel build and a new grant. Focused r8 executables
were built without rebuilding or signing the helper. Do not run `build.sh` or
`pairing-drill.mjs` here. The latter rebuilds helpers and initializes/removes
runtime state, so it is unsuitable for this retained-identity recheck.

## Unattended fence and real-window replay

```bash
set -e
cd /Users/miguel/chariox-worktrees/cumac-m2
export CUMAC_BUILD_DIR=/Users/miguel/.chariox/dev/cumac/build/r6
export PYTHONDONTWRITEBYTECODE=1
test "$(shasum -a 256 "$CUMAC_BUILD_DIR/Chariox Computer Helper.app/Contents/MacOS/helper" | awk '{print $1}')" = b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028
codesign --verify --strict "$CUMAC_BUILD_DIR/Chariox Computer Helper.app"
python3 apps/macos-computer-use/text-click-drill.py --suite
python3 apps/macos-computer-use/textedit-click-drill.py
python3 apps/macos-computer-use/fence-drill.py
```

The SHA-256 check fails unless the executable matches the pin. Stop on any
failure or missing grant. The foreign-cover drill independently binds the
foreign panel to its WindowServer ID, verifies it covers the selected point
without activation, and requires both explicit and pointer click refusals with
unchanged selection. It separately verifies stale-pointer refusal. The runners
honor GUI_HOLD/GUI_OK and close their own windows. A timeout never supplies an OS
grant. Preserve each `result.json`, including any failure. Do not retry mutation
as a way to turn a failure into PASS.

## Next matching build: unattended seat revoke and capture replay

The coordinator supplies `CUMAC_APPROVED_KERNEL_HOME` and
`CUMAC_APPROVED_KERNEL_PORT` for an existing, idle, approved durable kernel home
and its already registered endpoint. Before use, verify the approved public
fingerprints and protected backup/restore receipt under the key-retention
protocol. Do not choose a new port, copy an identity into task state, initialize
a new home, or delete existing state. This suite creates no identity and offers
no credential bootstrap. Those two deployment-specific values are required
inputs, not values the runner invents. Keep the home and all profiles protected.

For reference, the retained r6 kernel has SHA-256
`f28ef31f57e89d5eaffdb0f9728fe9ba4f41a2316160f8bc30c55242c6cfe2f9` and CDHash
`518b38b5cad455ee47274402ad86d0c20eda6d06`. The helper seals that exact kernel
allowlist. Source is `5ff2f8b862a57b9c0bc2428e080ad93f222e3d7e`. R8 source does not change this retained binary or its pairing contract.
Do not run the current strict cancellation oracle against it. It predates the
explicit cancellation outcome and cannot validate r8 source. The next recipe
requires coordinator-supplied paths and public SHA-256/CDHash pins from the
one combined r8 build after the pending r6 re-grant. Never substitute a key or
change a signed artifact to satisfy a pin.

Run this block only after that next matching build has its own grant, the
standalone suite passes against that exact helper, and durable-state checks pass.
It prepares the public fixture and performs the seat checks within the retained
fixture's 20-second watchdog. Ports for other listeners must be available on
this Mac. The local-only relay URL/token below are synthetic and cannot connect
to the hosted relay. No provider is run by this suite.

```bash
set -e
: "${CUMAC_APPROVED_KERNEL_HOME:?coordinator must supply the approved durable home}"
: "${CUMAC_APPROVED_KERNEL_PORT:?use its existing registered endpoint}"
test -d "$CUMAC_APPROVED_KERNEL_HOME"
: "${CUMAC_BUILD_DIR:?next granted helper build under the approved build root}"
: "${CUMAC_KERNEL:?next matching kernel under the approved build root}"
: "${CUMAC_KERNEL_SHA256:?public kernel pin from the combined build receipt}"
: "${CUMAC_HELPER_SHA256:?public helper pin from the combined build receipt}"
: "${CUMAC_HELPER_CDHASH:?public helper CDHash from the combined build receipt}"
case "$CUMAC_BUILD_DIR/" in /Users/miguel/.chariox/dev/cumac/build/*/) ;; *) exit 1;; esac
case "$CUMAC_KERNEL" in /Users/miguel/.chariox/dev/cumac/build/*) ;; *) exit 1;; esac
export CUMAC_BUILD_DIR PYTHONDONTWRITEBYTECODE=1
CUMAC_EVIDENCE=/Users/miguel/.codex/evidence/browser-computer-use-macos/owner-seat-$(date -u +%Y%m%dT%H%M%SZ)
test "$(shasum -a 256 "$CUMAC_KERNEL" | awk '{print $1}')" = "$CUMAC_KERNEL_SHA256"
test "$(shasum -a 256 "$CUMAC_BUILD_DIR/Chariox Computer Helper.app/Contents/MacOS/helper" | awk '{print $1}')" = "$CUMAC_HELPER_SHA256"
codesign --verify --strict "$CUMAC_BUILD_DIR/Chariox Computer Helper.app"
codesign --verify --strict "$CUMAC_KERNEL"
CUMAC_SCOPE=$(python3 apps/macos-computer-use/prepare-seat.py --evidence "$CUMAC_EVIDENCE")
CHARIOX_HOME="$CUMAC_APPROVED_KERNEL_HOME" \
CHARIOX_KERNEL_PORT="$CUMAC_APPROVED_KERNEL_PORT" \
CHARIOX_MACOS_COMPUTER_HELPER="$CUMAC_BUILD_DIR/Chariox Computer Helper.app" \
CHARIOX_MACOS_COMPUTER_HELPER_DRILL_CDHASH="$CUMAC_HELPER_CDHASH" \
CHARIOX_MACOS_COMPUTER_DRILL_SCOPE="$CUMAC_SCOPE" \
CHARIOX_RELAY_URL=ws://127.0.0.1:9 CHARIOX_RELAY_TOKEN=cumac-local-only \
"$CUMAC_KERNEL" >"$CUMAC_EVIDENCE/kernel.out" 2>"$CUMAC_EVIDENCE/kernel.err" &
CUMAC_KERNEL_PID=$!
trap 'kill -TERM "$CUMAC_KERNEL_PID" 2>/dev/null || true; wait "$CUMAC_KERNEL_PID" 2>/dev/null || true' EXIT
CUMAC_TOKEN="$CUMAC_APPROVED_KERNEL_HOME/state/kernel-local-auth/$CUMAC_APPROVED_KERNEL_PORT.token"
# Do not print or copy the token. Wait at most 5 s for the retained endpoint.
python3 - "$CUMAC_APPROVED_KERNEL_PORT" <<'PYREADY'
import socket, sys, time
end = time.monotonic() + 5
while time.monotonic() < end:
    try:
        with socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=.1): pass
        break
    except OSError: time.sleep(.05)
else: raise SystemExit('approved kernel endpoint not ready')
PYREADY
node apps/macos-computer-use/seat-revocation-drill.mjs \
  --port "$CUMAC_APPROVED_KERNEL_PORT" --token-file "$CUMAC_TOKEN" \
  --scope-file "$CUMAC_EVIDENCE/scope.json" --fixture-log "$CUMAC_EVIDENCE/fixture.out" \
  --evidence "$CUMAC_EVIDENCE"
kill -TERM "$CUMAC_KERNEL_PID"
wait "$CUMAC_KERNEL_PID"
trap - EXIT
```

Use a shell with `set -e` so any failed command stops the suite. Never launch
this block against an occupied endpoint or another kernel's live home. Do not
read logs back into chat without checking for private runtime contents.
Keep the kernel home after Stop. Cleanup only the launched kernel PID and the
public fixture whose retained executable, PID and scope receipt match. The
fixture watchdog closes it; the kernel runner always revokes its own native
seat on exit. Do not recursively delete a runtime directory.

Required verdicts in `result.json`:

- Live paired AX caret click reaches independent fixture index 126.
- Another supported AX caret click changes selection to 125. Paired protected
  PNGs differ, the actual r8 private attachment emits damage, and the independent
  JavaScript PNG decoder reconstructs every byte and verifies exact repair.
- The next caret click is independently observed at 126 while its request is
  pending. A second authenticated connection calls `revoke_grants`. Only the
  explicit `browser_action_cancelled` error can pass. Timeout, transport loss,
  helper refusal and owner-reset-required fail. Stop must acknowledge within
  2500 ms and the old seat must refuse further input without a changed selection.
  A completion race is FAIL. Preserve failure receipts and never replay mutation
  to manufacture PASS.

The pointer runner saves its restoration position after GUI wait and binds the
exact foreground PID/window/bounds. Every warp and jitter iteration checks that
binding and the last pointer position. Focus loss or human pointer movement
aborts further mutation and prevents restoration over the owner's interaction.
The runner closes only its own verified windows.

## Next matching build: shared encrypted native viewer

The kernel merge includes frozen display #950 at
`f04b4006b3b8b1b833560022128b0345db873e5a`. The Cloud reference is frozen
cloud#325 at `536bb5b1a`; it was not changed or live-tested here. R8 adds native
Computer display operations at local protocol 497 / relay peer 106. It uses
existing authenticated CXD1 transient delivery, without a new crypto stack.
The observer stays disabled unless `CHARIOX_MACOS_COMPUTER_DISPLAY=1` is supplied
to the next matching kernel. Local raw display requests and agent callers refuse.
Do not apply this flag to the retained r6 binary as a substitute for a new build.

The unattended browser entry point is
`apps/macos-computer-use/native-viewer-drill.mjs:runNativeViewerDrill`. Run it
through the coordinator's existing authenticated hosted relay transport and
approved fixture seat, using the shared `attachNativeDisplay` adapter. Required
inputs are the canvas, protocol-497 CXD1 transport, exact surface/string epoch,
and callbacks `sourcePNG`, `change`, `retire`. `sourcePNG` returns an independent
protected paired screenshot in memory; `change` performs one admitted bounded
caret click and verifies the independent fixture log; `retire` executes explicit
Revoke-all and returns `{revoked:true}` only after its acknowledgment. Supply no
keys, credential bootstrap, new home or new endpoint to the drill.

The drill compares every presented pixel with an independently decoded protected
capture after bootstrap, change and fresh-subscription repair. It requires
structured stale-epoch/not-granted/not-requested refusal after acknowledged
revocation. Timeout or transport failure cannot satisfy retirement. Catch the
fixed failure error and preserve its `.receipt`, including FAIL. Quiet-source
or blink differences fail exact comparison; do not loosen the oracle or retry
input to hide them. Close the stream and revoke the runner-owned seat on exit.

A synthetic transport, local replay, mocked canvas or public PNG fixture is only
component evidence. M2 acceptance additionally requires the granted new helper,
hosted encrypted Web/local-TUI/remote-TUI routes, required real app/site/provider
runs, protection canaries and the prescribed loss/reconnect/lock/sleep matrix.
The frozen Cloud viewer still needs coordinator integration of the native
adapter and exact target discovery. No native Cloud UI acceptance is claimed.

## Source fixes and remaining gates

R7 review fixes remain in source: same-process covers, exact build-launch receipt,
failed owned-release latch, invalid geometry retirement, secure-field
classification refusal, and bounded cleanup verdicts. R8 additionally fixes
#920 @bc53791b1 pointer takeover, #923 @8fc71e560 unconfirmed key release after
socket cancellation, and #957 @b5300f01f unsupported typing and generic-error
cancellation acceptance. The actual text-fixture scope maps click to AX text
caret input. Unconfirmed text/physical release latches before dispatch and
survives Stop/Start/reset. The owner must physically release input and explicitly
restart the kernel; restarting alone does not prove release succeeded.

All these changes are source-only relative to retained r6. Keep its helper
SHA-256 `b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028` and
CDHash `8b87ab7229c79a45f21eaddb7b2ef833095cf9d0` intact until the pending grant.
No helper bundle was rebuilt, re-signed or launched in r8. Do not run
`pairing-drill.mjs`; it rebuilds helpers and initializes/removes runtime state.
The older bundle CDHashes remain
`d171e9751ee725169cdb777d6ffc3c46e6bdd86b` and
`425753b7ce44dd772d2e428f3bc36b634d69eaeb`.

M0's previous aggregate FAIL remains. Existing click/scroll/capture and M1 PASS
receipts remain historical. M2 is BLOCKED pending the matching grant and live
matrix. M3 physical holds/human observation/fatal reset, M4 AX/OCR/private
coverage and M5 codec/release/full client matrix remain pending. Preserve
runtime homes, credentials, public history and all evidence outside Git.
