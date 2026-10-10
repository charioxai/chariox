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

After the re-grant, the coordinator runs the following suite. It uses the
retained r6 bundle, not the newer r7 helper source. The r7 focused executables
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

## Unattended kernel-seat revoke and M2 capture/damage replay

The coordinator supplies `CUMAC_APPROVED_KERNEL_HOME` and
`CUMAC_APPROVED_KERNEL_PORT` for an existing, idle, approved durable kernel home
and its already registered endpoint. Before use, verify the approved public
fingerprints and protected backup/restore receipt under the key-retention
protocol. Do not choose a new port, copy an identity into task state, initialize
a new home, or delete existing state. This suite creates no identity and offers
no credential bootstrap. Those two deployment-specific values are required
inputs, not values the runner invents. Keep the home and all profiles protected.

The matching retained kernel must have SHA-256
`f28ef31f57e89d5eaffdb0f9728fe9ba4f41a2316160f8bc30c55242c6cfe2f9` and CDHash
`518b38b5cad455ee47274402ad86d0c20eda6d06`. The helper seals that exact kernel
allowlist. Source is `5ff2f8b862a57b9c0bc2428e080ad93f222e3d7e`. The r7 kernel
sources do not change this binary or its pairing contract.

Run this block only after the standalone suite above and durable-state checks.
It prepares the public fixture and performs the seat checks within the retained
fixture's 20-second watchdog. Ports for other listeners must be available on
this Mac. The local-only relay URL/token below are synthetic and cannot connect
to the hosted relay. No provider is run by this suite.

```bash
set -e
: "${CUMAC_APPROVED_KERNEL_HOME:?coordinator must supply the approved durable home}"
: "${CUMAC_APPROVED_KERNEL_PORT:?use its existing registered endpoint}"
test -d "$CUMAC_APPROVED_KERNEL_HOME"
export CUMAC_BUILD_DIR=/Users/miguel/.chariox/dev/cumac/build/r6
export PYTHONDONTWRITEBYTECODE=1
CUMAC_KERNEL=/Users/miguel/.chariox/dev/cumac/build/r6-retained-kernel/chariox-kernel
CUMAC_EVIDENCE=/Users/miguel/.codex/evidence/browser-computer-use-macos/owner-seat-$(date -u +%Y%m%dT%H%M%SZ)
test "$(shasum -a 256 "$CUMAC_KERNEL" | awk '{print $1}')" = f28ef31f57e89d5eaffdb0f9728fe9ba4f41a2316160f8bc30c55242c6cfe2f9
codesign --verify --strict "$CUMAC_KERNEL"
CUMAC_SCOPE=$(python3 apps/macos-computer-use/prepare-seat.py --evidence "$CUMAC_EVIDENCE")
CHARIOX_HOME="$CUMAC_APPROVED_KERNEL_HOME" \
CHARIOX_KERNEL_PORT="$CUMAC_APPROVED_KERNEL_PORT" \
CHARIOX_MACOS_COMPUTER_HELPER="$CUMAC_BUILD_DIR/Chariox Computer Helper.app" \
CHARIOX_MACOS_COMPUTER_HELPER_DRILL_CDHASH=8b87ab7229c79a45f21eaddb7b2ef833095cf9d0 \
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

- Live paired caret reaches the independent fixture index 126.
- Two paired protected PNGs show the bounded public `r7` text change. The actual
  r7 private attachment replays them into native PNG/tiles and exact repair.
  The independent shared JavaScript PNG decoder reconstructs the damage and
  compares every byte against the changed capture.
- A second caret Action changes selection to 125 while its request is still
  pending. A second authenticated connection calls `revoke_grants`. The input
  must cancel, Stop must acknowledge within 2500 ms, and the old target must
  refuse the next input with selection unchanged after acknowledgment. A
  completion race is FAIL, not mid-input acceptance and not a reason to replay.

The capture/damage replay uses the real paired r6 captures and the compiled r7
private attachment, so it is supplementary evidence. It does not validate r7
SCK callbacks in the helper or encrypted shared-viewer delivery. For full M2,
allocate the native surface/display union, connect caller-bound kernel and
presenter registration through the existing encrypted transient event route,
batch one next helper rebuild and new grant, then validate the exact new source
with the hosted relay/Web/TUI. Never represent the local replay as that gate.

R7 source is feature-disabled and not ready to stage. M3 physical input/owned
holds and fatal reset, M4 AX/OCR/private-region coverage, and M5 codec/release
and full client/relay matrix remain pending. The older bundles retain CDHashes
`d171e9751ee725169cdb777d6ffc3c46e6bdd86b` and
`425753b7ce44dd772d2e428f3bc36b634d69eaeb`. R5's foreign-cover failure remains
historical until the exact r6 helper clears it. Evidence remains outside Git.
