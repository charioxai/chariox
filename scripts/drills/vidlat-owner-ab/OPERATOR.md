# MP-08 / MP-10 / MP-11 — public prebuilt owner A/B kit

The quiet CCX43 runs the two browser pipelines. Chariox uses the **existing
staging web app and the relay returned by its normal product device flow**.
Selkies uses the owned b3 stack's private Caddy peer gate. No validation-origin
Auth0/WorkOS registration or identity-provider change is required. Raw
kernel/CDP ports stay loopback; raw Selkies admits only the b3 gateway and
loopback. No provider accounts or operator keys are shipped or linked.

**MP-08/MP-10 activation is BLOCKED.** Named staging Cloud
`7af6207c746b319db8c874ac2ac8704bc3eaeb44` does not contain
`apps/web/src/kernel-browser/display.ts` or its video sink. OSS
`f57ac8f7cd2908b2b76205b3f8fb344223fa6a06` is local490/peer100 and lacks the
`KernelBrowser` request/native-display path. r8 uses local498/peer107 with
Cloud1921b7cc's viewer. Building staging main cannot supply that video path.
Direct Node requests reach the staging device-start/poll endpoints. Browser
and health routes redirect to Cloudflare Access, so their unauthenticated
responses cannot establish the deployed Cloud/relay version. Python urllib
denials were transport-specific; do not treat them as machine admission proof.

Coordinator actions before READY: provide reviewed compatible staging
viewer/runtime/relay pins. Direct staging product device admission is reachable;
keep its supported path and existing browser identity unchanged. Then rehearse
the same pins on a lane-owned
fixture stack and verify the staged deployment read-only. Do not alter staging,
Omarchy or a shared login from this kit. No paid host until Miguel says A/B ready.
The artifact manifest's `staging_qualification.status` stays BLOCKED until that
proof exists; owner startup rejects unqualified artifacts and gated APIs.

1. Rent a fresh quiet **CCX43, ubuntu-26.04, x86_64** when Miguel is ready.
   Transfer this public kit archive and verify its recorded SHA-256. Extract to
   `/root/.chariox/dev/vidlat/owner-ab-kit`; keep evidence outside that directory.
   Use the coordinator's approved SSH access; the kit generates no SSH keys.
2. On b3 start only the Selkies gate stack `pr972-vidlatr9stage` with
   `/w/valenv/pr-stack start` and put
   the planned end epoch in its KEEP file. Add the new host's temporary peer
   allowance with `/w/valenv/scripts/peer-allow add IP MINUTES vidlat-owner-ab`.
   Register Miguel's current network IP with that helper if it has changed.
3. On the host export these **public settings** and run `ab-up.sh`:

   ```bash
   export AB_STATE=/root/.chariox/dev/vidlat/owner-ab-20261011
   export AB_ORIGIN=https://staging.chariox.com
   # Set AB_API_ORIGIN only if the coordinator supplies a supported machine API.
   # Default: AB_API_ORIGIN=$AB_ORIGIN. Device approval must still use staging.
   # AB_RELAY_URL is optional: assertion only, never an override of Cloud.
   unset AB_RELAY_URL
   export AB_SELKIES_ORIGIN=https://pr972-vidlatr9stage-selkies.val.51-255-87-178.sslip.io
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
   python3 /path/to/kit/ab-gate.py attach pr972-vidlatr9stage HOST_IP STREAM_PORT
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
   records an idle default agent; the kit sends no provider prompt. Give Miguel
   `OWNER_AB_TEST.md` only after staging compatibility and machine admission
   have passed. His existing staging sign-in is reused; he approves the one
   product device challenge. Never copy an existing machine profile into the kit.

Teardown, even after an interrupted run:

```bash
# On the CCX43, with the SAME public environment settings:
/root/.chariox/dev/vidlat/owner-ab-kit/ab-down.sh
# On b3:
python3 /path/to/kit/ab-gate.py detach pr972-vidlatr9stage
/w/valenv/scripts/peer-allow del HOST_IP
rm /w/valenv/prs/pr972-vidlatr9stage/KEEP
/w/valenv/pr-stack stop pr972-vidlatr9stage
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
optionally `AB_SKIP_INSTALL=1` when dependencies are already installed.
With compatible paired sources it
proves script/runtime lifecycle, real built app video and gated WSS routing;
it cannot establish fresh-host install duration, Mac/Retina/home-network UX,
strict latency/CPU/FPS parity, provider/Vault acceptance or multi-hour stability.
Keep those qualifications with every artifact hash and readiness report.
