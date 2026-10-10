# Standalone M0 recheck, with a pinned integration prerequisite

M0 provides standalone signed-helper capture/AX drills. It does not implement
kernel pairing, protected shared-viewer attachments or kernel grant revocation.
The selected-visible-window fence now refuses any positive-alpha covering
window with a different window ID, including one in the selected process.

Do not rebuild or re-sign the preserved helper while Miguel's r6 re-grant waits.
The combined r6 bundle at
`/Users/miguel/.chariox/dev/cumac/build/r6/Chariox Computer Helper.app` has
CDHash `8b87ab7229c79a45f21eaddb7b2ef833095cf9d0` and SHA-256
`b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028`.
It includes the earlier foreign-process cover fence, but not the new r7
same-process cover fix. That fix is batched for the next combined rebuild.

After its re-grant, this source supplies the standalone procedure and cover
runner itself. No owner pointer placement or fixture clicking is required.

```bash
set -e
cd /Users/miguel/chariox-worktrees/cumac-m0
export CUMAC_BUILD_DIR=/Users/miguel/.chariox/dev/cumac/build/r6
export PYTHONDONTWRITEBYTECODE=1
test "$(shasum -a 256 "$CUMAC_BUILD_DIR/Chariox Computer Helper.app/Contents/MacOS/helper" | awk '{print $1}')" = b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028
codesign --verify --strict "$CUMAC_BUILD_DIR/Chariox Computer Helper.app"
python3 apps/macos-computer-use/text-click-drill.py --suite
python3 apps/macos-computer-use/textedit-click-drill.py
python3 apps/macos-computer-use/fence-drill.py
```

The runners select helper and fixture from CUMAC_BUILD_DIR. Every helper launch
checks its receipt against the selected executable path and public CDHash,
signature validity and LaunchServices parent PID 1. Wrong-build evidence fails.
The cover runner proves a foreign nonactivating panel covers the selected point
without changing frontmost PID, requires explicit/pointer cover refusal with
unchanged selection, and separately verifies a stale-pointer refusal. Failed
owned-window cleanup changes the final verdict to FAIL. Honor GUI_HOLD/GUI_OK
and stop on missing grants. Evidence stays under the owner evidence root.

This r6 live rerun can clear the historical foreign-process failure. It cannot
validate the new same-process fence or establish the complete M0 gate. Keep
those gates blocked until the next batched exact helper passes the corresponding
live cases. No r7 GUI/input or helper rebuild has occurred.

The combined pairing/capture source is separately published in
[draft #957 at 777b256c4](https://github.com/charioxai/chariox/tree/777b256c41875202ad0921d33aac5c40541e946a/apps/macos-computer-use).
Kernel pairing/revocation instructions belong to that integration prerequisite,
not this standalone helper. The coordinator's newer local M2 OWNER_RECHECK.md
contains the revised mid-input/capture-damage suite; it remains unpublished.
Never substitute pairing-drill.mjs here, since it rebuilds helpers and initializes
and removes runtime state. Retain durable identities, profiles, keys, backups,
legacy relay state and reviewer infrastructure. No recursive state cleanup.
