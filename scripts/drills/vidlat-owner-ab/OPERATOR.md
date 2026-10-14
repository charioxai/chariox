# MP-08 / MP-10 / MP-11 — private validation-origin A/B kit

Use the frozen viewer stack `pr972-vidlatab` (Cloud1921b7cc05, OSSae4ffec402,
local498/peer107). Chariox and the qualified Selkies wheel run side by side on
one quiet CCX43, Ubuntu26.04. The stack's existing trusted dev identity is
`pr972-vidlatab@chariox.invalid`; its private Caddy gate admits Miguel's home IP.
No identity-provider dashboard, real owner account or shared provider login is
used. The fixture identity is disposable, and needs no cookie/token transfer.

**Exact owner sign-in step:** open
https://pr972-vidlatab.val.51-255-87-178.sslip.io/waiting-room .
Existing stack dev auth automatically selects its fixture account. The owner
never handles a kernel device code; the coordinator completes that enrollment
before handing over the two links in OWNER_AB_TEST.md.

1. When Miguel is ready, rent the authorized quiet CCX43 and transfer the
   prebuilt public archive. Verify its SHA256 and extract it to
   `/root/.chariox/dev/vidlat/owner-ab-kit`. No builds or private operator keys
   are included. Start only `pr972-vidlatab` using `/w/valenv/pr-stack start`.
   Write the planned end epoch to `/w/valenv/prs/pr972-vidlatab/KEEP`, and add
   the quiet host IP with `/w/valenv/scripts/peer-allow add IP MINUTES vidlat-owner-ab`.
2. On the quiet host:

   ```bash
   export AB_STATE=/root/.chariox/dev/vidlat/owner-ab-owner
   export AB_ORIGIN=https://pr972-vidlatab.val.51-255-87-178.sslip.io
   export AB_SELKIES_ORIGIN=https://pr972-vidlatab-selkies.val.51-255-87-178.sslip.io
   export AB_GATEWAY_IP=51.255.87.178
   export AB_EVIDENCE=/root/.codex/evidence/browser-resume-20260930/vidlat/owner-ab
   unset AB_API_ORIGIN AB_RELAY_URL AB_REHEARSAL AB_SKIP_INSTALL
   /root/.chariox/dev/vidlat/owner-ab-kit/ab-up.sh
   ```

   Startup verifies shipped file hashes, exact authorized fixture/source
   qualification, machine API availability, public dependencies and direct
   Cisco OpenH264 download. It enforces a 900-second startup budget. Raw kernel
   and CDP ports bind loopback; Selkies admits only loopback and the b3 gateway.
3. Read only `streamPort` from the host's `runtime.json`. On b3 attach the
   owned Selkies route:

   ```bash
   python3 /path/to/kit/ab-gate.py attach pr972-vidlatab HOST_IP STREAM_PORT
   ```

   The helper serializes through the valenv lock, validates/reloads Caddy and
   preserves the private gate and four-hour WebSocket close delay.
4. On the quiet host run `node /path/to/kit/ab-control.mjs login`. It writes
   its private device challenge locally with mode0600. From an admitted
   coordinator browser open that record's `verification_url` and click
   **Approve** through the normal product page. The stack applies only its
   disposable fixture identity. Never print, put in chat/evidence or hand the
   private device link to Miguel. Normal five-second polling connects the
   kernel to the Cloud-issued WSS relay and creates the ordinary `owner-ab`
   session. Read only the public IDs in `selection.json` for validation.
5. Verify fresh ONLINE heartbeat and that a fresh browser opens both video
   surfaces, then give Miguel OWNER_AB_TEST.md. Only his ratings remain.

Always tear down with the same public environment settings:

```bash
# quiet host
/path/to/kit/ab-down.sh
# b3
python3 /path/to/kit/ab-gate.py detach pr972-vidlatab
/w/valenv/scripts/peer-allow del HOST_IP
rm /w/valenv/prs/pr972-vidlatab/KEEP
/w/valenv/pr-stack stop pr972-vidlatab
```

Cleanup checks positive PID/start identities, removes only marked disposable
state, its own workspace and exact firewall rule, and leaves a numeric receipt.
The coordinator deletes the rented host. Keep the protected stack config/data
in place; never prune Docker, shared provider profiles or reviewer state.

MP-10 rehearsal uses `AB_REHEARSAL=1` (loopback Selkies) on b3;
`AB_SKIP_INSTALL=1` is permitted only with installed dependencies. It exercises
fresh browser profiles at DPR1/2, real built app/kernel and hostedWSS with
public sites, plus coordinator enrollment and one-step fixture access.
It is kit readiness, not Mac/home-network ratings, strict parity, provider/Vault
acceptance, multi-hour stability or a fresh quiet-host installation result.
The frozen display P2 backlog remains deferred by coordinator direction.
