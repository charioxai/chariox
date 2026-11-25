# vault-d3: test-only harness, fixtures and RED drills for the post-L14 Vault/handover rows

Rows 545 546 547 549 551 557 558 (D3 handover card, D1/D2 never-seen). Nothing here changes product code. Design: vaultA0 `HARNESS.md`, `CELLS.md`.

| Dir | What |
|---|---|
| `never-seen/` | The "never seen" scan (`never-seen.mjs`): needle from stdin or a file, never printed; scans files (jsonl, ndjson, json, log, sqlite/wal, any bytes), base64 image blocks and screenshots through OCR (tesseract, in memory), and `/proc/*/cmdline` + `environ`. Needle forms: raw, base64 (3 alignments), base64url, hex, URL, JSON-escaped, first/last 12. Refuses empty, <16 chars, one repeated char. Controls: synthetic (every file type, chunk boundary, OCR) and live (`--live-control-file`). Exit 0 CLEAN, 1 LEAK, 2 INVALID. `roots.mjs` builds scan roots per kernel. |
| `fixtures/` | `serve.mjs` starts seven owned origins: F1 sign-up (`?eye=1`, `?big=1`), F2 reveal / copy / QR token page (`/show` serves the value only on click; `/verify` logs sha256 only; `?swap=1`), F3 login form that navigates to `/welcome` (cross-site with `--alt-host`) + `/small` 300x75 box, F5 OAuth relying party with a `window.open` popup and its own IdP, F4 human-check page with our own cross-origin canvas grid iframe (vendor hCaptcha/Turnstile test widgets only with `?vendor=1`). Issued tokens go to `--state-dir` (mode 600). |
| `person/` | `person-driver.mjs`: Playwright person at `desktop-dpr1`, `desktop-dpr2`, `phone-dpr2`; real key typing; `regionFacts` (box vs viewport, colour count of the region so "black" is a number); `cardTimeline` for the two-client cell. Sign-in is a vendored storageState file (`CHARIOX_PC_STORAGE_STATE`), not the host-local `/w/valenv` helper. |
| `red/` | `kernel-layer-drill.mjs <545\|546\|547>`: real `KernelBrowserHost` + Chrome. `551-restart-kernel.mjs`: static contract. `host-harness.mjs`: the harness. |

## Running
- Unit tests (any host): `node --test scripts/drills/vault-d3/never-seen/never-seen.test.mjs scripts/drills/vault-d3/fixtures/fixtures.test.mjs`
- Person driver on the fixtures: `PW_FROM=<node_modules with playwright-core> node --test scripts/drills/vault-d3/person/person-driver.test.mjs` (needs a Chrome that starts: not an agent shell inside Bubblewrap).
- Kernel-layer drills need a host that is NOT inside Bubblewrap with Xvfb, openbox, dbus-daemon, setxkbmap, python3-xlib, python3-pil, python3-av and Chrome:
  `CHARIOX_MDACCESS_DRILL_ROOT=<dir> CHARIOX_KERNEL_BROWSER_EXECUTABLE=/usr/bin/google-chrome node scripts/drills/vault-d3/red/kernel-layer-drill.mjs 545`
  `D3_ORIGINS_FILE=<origins.json of a running serve.mjs>` points it at hosted fixtures instead of local ones.
- Never-seen over a kernel: write `roots.json` (see `roots.mjs`), then `printf %s "$VALUE" | node never-seen.mjs --roots roots.json --needle-stdin --ocr --out scan.json` (value via `chariox.vault.use_env` stdin; never in argv).
- Fixture host (Boat): `node fixtures/serve.mjs --state-dir <dir> --host :: --public-host <name> --alt-host alt.<name> --tls-key K --tls-cert C`.

## Live Vault cells (rows 557 / 558): `live/`
`live/run-live.mjs --cells d1,d2 --evidence /root/.chariox/dev/<lane>/evidence/<run>` starts a drill kernel (`drill-kernel.mjs`: own CHARIOX_HOME, ports and HOME; Vault created and unlocked; real Claude installed through `SetProviderAccountCredential` from `/root/.claude-lanes/oauth-token.chariox`, read in-process, never printed), restarts the fixtures on the Boat (`boat.mjs`), gives the agent the D1 / D2 task, then scans with the fixture's own needle piped to the scanner's stdin. Run it inside a host unit so the kernel browser can start:
`systemd-run --wait --collect -p MemoryMax=8G -E D3_CLIENT_DIST=<built packages/kernel-client/dist> -E D3_KERNEL_BIN=<chariox-kernel> node live/run-live.mjs ...` (watch it with a kernel process wake). Output: `cells.json`, one `*.scan.json` per needle, the agent's `*-report.txt`, the session-state export. Delete the run's state dir and the needles dir afterwards.

### Per-surface live decoys
A scan is trusted on a surface only when that surface's OWN live decoy is found there in the same run (`never-seen/surface-scan.mjs`: group x surface matrix; BLOCKED when the decoy is not found, FAIL when a real secret has a hit). `run-live.mjs` plants: a chat decoy in the prompt (kernel state, provider session), a shell decoy (`echo`), a file the drill writes into the workspace, a decoy in the Room's workspace path (the one value of ours the kernel log writes: it logs command types, ids and paths, no free text), a decoy bad credential id (tool-error text), a card with a decoy message (session export), a canvas on the fixture page that draws a decoy as pixels (screenshot, OCR), and a helper process with a decoy in argv and another in environ, alive during the scan. Synthetic controls do not count for these surfaces.
