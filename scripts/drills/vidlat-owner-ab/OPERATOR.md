# MP-08 / MP-10 / MP-11 — public prebuilt owner A/B kit

The CCX43 runs only the two browser pipelines. The Chariox web app and relay
use the owned, exact-head b3 stack `pr972-vidlatr8ab` and hosted HTTPS/WSS.
Selkies is proxied through that stack's existing valenv **private** Caddy peer
gate. Raw kernel/CDP ports stay loopback. Raw Selkies admits only the b3 gateway
and local loopback. No provider profiles, private operator keys or passwords are
in this kit. No build tool or source build runs on the rented host.

Before renting, the coordinator must configure this own stack for **normal
product sign-in**, registering its origin with the existing Auth0/WorkOS
tenant, permitting Miguel's own email, disabling fixture auth, and using the
stock production API entry rather than `cloud-entry.mjs`. Existing valenv
fixture login is only suitable for the b3 rehearsal. Do not issue owner links
until the normal sign-in and exact hosted WSS connection are verified. Auth
configuration remains in protected operator config, supplied out of band.

1. Rent a fresh quiet **CCX43, ubuntu-26.04, x86_64** when Miguel is ready.
   Transfer this public kit archive and verify its recorded SHA-256. Extract to
   `/root/.chariox/dev/vidlat/owner-ab-kit`; keep evidence outside that directory.
   Use the coordinator's approved SSH access; the kit generates no SSH keys.
2. On b3 start only `pr972-vidlatr8ab` with `/w/valenv/pr-stack start` and put
   the planned end epoch in its KEEP file. Add the new host's temporary peer
   allowance with `/w/valenv/scripts/peer-allow add IP MINUTES vidlat-owner-ab`.
   Register Miguel's current network IP with that helper if it has changed.
3. On the host export these **public settings** and run `ab-up.sh`:

   ```bash
   export AB_STATE=/root/.chariox/dev/vidlat/owner-ab-20261011
   export AB_ORIGIN=https://pr972-vidlatr8ab.val.51-255-87-178.sslip.io
   export AB_RELAY_URL=wss://pr972-vidlatr8abrelay.val.51-255-87-178.sslip.io
   export AB_SELKIES_ORIGIN=https://pr972-vidlatr8ab-selkies.val.51-255-87-178.sslip.io
   export AB_GATEWAY_IP=51.255.87.178
   export AB_EVIDENCE=/root/.codex/evidence/browser-resume-20260930/vidlat/owner-ab
   /root/.chariox/dev/vidlat/owner-ab-kit/ab-up.sh
   ```

   Startup verifies every shipped file, installs only public runtime packages,
   fetches Cisco OpenH264 directly, starts the real kernel and qualified
   Selkies, checks readiness and enforces the 900-second budget. Human login
   time is separate. Package downloads require internet access. A failure
   settles exact owned processes and removes the disposable state/rule.
4. Read only `streamPort` from the host's `runtime.json` with a script; do not
   display any controller, auth, Vault or device-login file. On b3 run:

   ```bash
   python3 /path/to/kit/ab-gate.py attach pr972-vidlatr8ab HOST_IP STREAM_PORT
   ```

   This helper serializes through the valenv stack lock, changes only this
   stack's Caddy fragment and validates/reloads Caddy. It preserves the existing
   gate and four-hour stream-close delay. No shared Caddy file is edited.
5. Run `node /path/to/kit/ab-control.mjs login` on the host. The normal product
   device challenge is written mode0600 to `device-login.private.json`; use
   that URL in the owner's authenticated product browser without copying it
   to chat/evidence. The script polls at the product's five-second cadence,
   connects through the real hosted WSS relay, creates a normal **owner-ab**
   session in its own workspace under `/root/work`, and records public
   Machine/Kernel/Session selectors in `selection.json`. Session creation
   records an idle default agent; the kit sends no provider prompt. Give Miguel `OWNER_AB_TEST.md` after proving
   normal sign-in, fresh ONLINE target and both live video surfaces.

Teardown, even after an interrupted run:

```bash
# On the CCX43, with the SAME public environment settings:
/root/.chariox/dev/vidlat/owner-ab-kit/ab-down.sh
# On b3:
python3 /path/to/kit/ab-gate.py detach pr972-vidlatr8ab
/w/valenv/scripts/peer-allow del HOST_IP
rm /w/valenv/prs/pr972-vidlatr8ab/KEEP
/w/valenv/pr-stack stop pr972-vidlatr8ab
```

`ab-down` inventories only the recorded supervisor and its verified descendants,
checks start ticks before every positive-PID signal, and refuses PID 0/1/-1.
It verifies no live child remains, removes its exact firewall rule and deletes
only its explicit disposable state, including product-generated identities and its marked A/B workspace.
It retains a numeric cleanup receipt under AB_EVIDENCE. No global prune,
process-name kill or cleanup of provider/shared/reviewer state occurs. Finally
the coordinator deletes the rented host through the ordinary infrastructure
path. Remove only this transferred public kit if no longer needed.

MP-10 rehearsal is run on b3 with `AB_REHEARSAL=1` (Selkies loopback-only),
optionally `AB_SKIP_INSTALL=1` when dependencies are already installed. It
proves script/runtime lifecycle, real built app video and gated WSS routing;
it cannot establish fresh-host install duration, Mac/Retina/home-network UX,
strict latency/CPU/FPS parity, provider/Vault acceptance or multi-hour stability.
Keep those qualifications with every artifact hash and readiness report.
