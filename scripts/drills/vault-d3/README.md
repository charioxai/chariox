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
