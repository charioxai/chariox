# LOCALFAST forged-client harness (test tool)

A Node client that does what the Cloud web page does to open a same-machine direct carrier to a kernel (real Cloud sign-in, real 30 s relay
lease token, encrypted `local_browser_connect` over the real relay, then the loopback WebSocket or WebRTC data-channel handshake) and
forges exactly one field per cell. Every forged cell is read next to an unforged control (F0). No product code is touched.

| Cell | Forged |
| --- | --- |
| F0 | none (control, WebSocket + WebRTC) |
| F1 | Origin (missing, null, other site, look-alikes, duplicate), Host, proxy headers, query, path; WebRTC proof origin; real foreign page |
| F2 | stolen grant on another key, proof to the wrong kernel key, tampered/garbled/oversized proof, relay envelope by another key, silent client |
| F3 | other kernel id, grant of kernel A at kernel B, token for another target, unknown kernel, wrong kernel key |
| F4 / F4r | forged epochs; the real epoch change after a kernel restart |
| F5 | grant reuse, captured proof replay, WebSocket grant on a data channel, same SDP offer twice, second `local_connect` |
| F6 | grant older than 30 s, expired lease token, 15-minute bootstrap token, lease not renewed, renewal replay / other key / over the direct socket |
| F7 | localhost impostor at the far end of the real web client's data channel (in-page shim; the real kernel still mints the real grant) |
| F8 | LogoutCloudRelay while a burst of CreateSession is queued on an admitted carrier |
| F9 | two owners on two private stacks, one drill kernel and Vault entry each: cross-owner reveal/replace, relay, grants, local CLI listener |

## Running (private stack + drill kernels, never live kernels)
1. `pr-stack up <name>` twice (stacks A and B; B only for F9) at the exact Cloud and OSS heads; drill kernels with `env -i`-style units
   (own `CHARIOX_HOME`, own ports), four of them: k1/k2/k3 on stack A, k4 on stack B.
2. `node setup.mjs <stack> <kernel-port> <kernel-dir> <alias> <state.json>` pairs each kernel (real device approval).
3. `node run-cells.mjs <evidence-dir> state-k1.json state-k2.json state-k3.json [cell ids]` (`state-k4.json` is read next to `state-k1.json`).
   It must run in a systemd unit (the stack's sign-in key is only readable there). Results: `<evidence-dir>/cells.json` + `cells.log`.

Dependencies: Playwright (`/w/valenv/tools/node_modules/playwright-core`), Chrome, `ws`, and the built `packages/kernel-client/dist`.
Test Vault entries hold throwaway markers; the drill Vault passphrases are random and live only in the harness process.
