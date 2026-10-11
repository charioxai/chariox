# One owner session for the matching Mac build

R14 source handoff, 2026-10-11. This session is BLOCKED until the coordinator
supplies the matching combined build and the real integration callbacks. It is
not staged. Miguel's morning K re-grant remains pending. The retained r6 helper
has not been rebuilt, signed, replaced or launched during source work.

Retained r6 identity, historical replay only:

- Build `/Users/miguel/.chariox/dev/cumac/build/r6/Chariox Computer Helper.app`.
- Identifier `ai.chariox.computer-helper`.
- CDHash `8b87ab7229c79a45f21eaddb7b2ef833095cf9d0`.
- Executable SHA-256 `b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028`.

That binary predates M3/M4 and current review fixes. Re-granting it can replay
historical failures but cannot pass the combined suite. For one owner session,
the coordinator must select and pin the final combined M0-M4 build before the
appointment. Keep r6 intact at its existing path. Do not silently substitute a
new helper in that bundle or ask for separate milestone grants.

## Before Miguel starts

Coordinator prerequisites are the current M0-M4 source plus M5 codec attachment,
a matching validated kernel, authenticated Mac Vault plain-text field mapping,
private UI exclusions, real native Cloud/TUI integration and admitted transports.
Supply the independent public fixture/native event receipts and fatal press
hook. M4 opaque AX, query and language coverage remains incomplete. These are
blocking source/integration dependencies. Headless tests cannot clear them.
Whole-kernel tests belong on b3. Do not run them on this Mac.

R13 closes all three findings in #964 @6101151bd. Native subscribe conforms to
protocol 497 with only target/codecs; browser bitrate options are omitted and
the kernel keeps the native bitrate. The unsigned owner build runs ad-hoc signed
policy tests before linking unsigned release bundle executables. Before the
appointment, run `check-codec.sh` with fresh `CUMAC_BUILD_DIR` and
`CUMAC_EVIDENCE_DIR` paths under the approved build/evidence roots. The driver
creates the evidence directory and supplies one artifact path to both encoder
and independent decoder. Record both PASS lines. These component checks do not
clear the matching-build, signing, grant or real-route acceptance gates.

All builds live under `/Users/miguel/.chariox/dev/cumac/build`, one Swift build
at a time with `-j2`. Use a fresh bundle path, verify public helper/kernel SHA-256
and CDHash pins, and preserve the granted helper. No installed bundle launch,
sudo, agent TCC request, Keychain access or new credentials are authorized.

The approved durable kernel home and existing registered endpoint come from the
coordinator. Before first credential use, verify public fingerprints and the
protected backup/tested restore receipt. Do not initialize another identity,
copy credentials into task state, print tokens or delete a runtime home. Keep
`~/.chariox/keys/`, `~/.chariox-key-backups/`, provider profiles, reviewer state
and the legacy relay key directory outside all cleanup. Hosted relay access
uses the verified durable configuration; no relay credentials are changed here.

## Miguel's single appointment

1. If Developer ID readiness is confirmed, Miguel personally performs the fresh
   unsigned build/sign/notarize commands in
   `docs/COMPUTER_USE_MACOS_M5_SIGNING.md` before the grant. Check Team, strict
   signature validity, Accepted notarization and stapling. Record public pins.
   If signing is not ready, record M5 release BLOCKED; do not access Keychain
   as an agent or describe the ad-hoc build as a notarized release.
2. Miguel grants the one pinned matching helper Accessibility and Screen
   Recording in System Settings. If the M3 observer separately needs Input
   Monitoring, Miguel chooses that opt-in during the same appointment. No grant
   to a provider, shell, Terminal or kernel is a substitute. Verify launch/TCC
   attribution through the approved kernel launch receipt. Stop on attribution
   mismatch or nonprompting preflight denial.
3. Leave the build unchanged. The coordinator runner takes over fixture setup,
   all checks and exact owned cleanup below. No pointer placement, document
   opening, fixture clicking or separate per-milestone appointment is required.

Signing must precede the grant because it changes the helper CDHash. Never
rebuild, re-sign, edit sealed metadata or change its kernel allowlist afterward.
If the only ready artifact is r6, retain the historical result and leave the
combined gate BLOCKED. Do not claim it validates later source.

## Unattended order after that grant

The coordinator uses existing `prepare-seat.py`, caret/fence scripts,
`seat-revocation-drill.mjs`, `native-viewer-drill.mjs` and
`native-input-drill.mjs` with real approved transports and fresh fixture receipts.
No generic transport/fatal hook is invented by this lane. M4 orchestration and
real client adapters remain required. Preserve every failed receipt; no mutation
retry may manufacture PASS.

1. M0, both caret runners and foreign/same-process cover fences. Verify current
   selection independently. Focus/pointer/window takeover after the Python
   precheck must cause zero native warps. Returned focus cannot revive cleanup.
   Require all runner-owned windows closed within their bounded deadlines.
2. M1, paired identity, lease, Stop/revoke, stale epochs and strict in-flight
   cancellation. Only `browser_action_cancelled` proves cancellation. Timeout,
   transport loss or `owner_reset_required` fails. Oversized/unsupported/no-effect
   input must leave subsequent admitted input usable. Lost release confirmation
   must remain latched across Stop/Start.
3. M2, protected pixels, exact PNG/tile reconstruction and fresh-subscription
   repair through real encrypted Web/local-TUI/remote-TUI routes. Independent
   protected captures must match all presented pixels. Revocation acknowledgment
   must precede the structured stale/not-granted/not-requested denial.
4. M3, healthy local-human observer, admitted native input, physical/remote
   takeover, denied input with unchanged independent selection and explicit
   safe release/resume. First claim Desktop by terminal takeover and completed
   terminal input, then exercise idle physical activity and physical interruption
   of terminal move/pre-down click. The authenticated owner terminal must reset,
   release and resume that same generation. Supply distinct `click`/`resumeClick`
   destinations and distinct `expectedSelection`/`expectedResumeSelection`
   values. Independently read selection before and after each dispatch; an
   already correct selection or acknowledged no-effect input must fail.
   Require explicit `browser_action_cancelled`, successful reset and release,
   and continued use of that exact pairing generation. A safely released local
   pause must not disconnect; terminal/grant authority retirement must still
   disconnect. Failed helper reset must leave Desktop owned and input fenced.
   Defer fatal owned-press death until the end.
5. M4, existing snapshot/OCR/target_action paths on the signed public fixture.
   Independently prove the button counter. Refuse refs stale after layout/focus,
   observer changes, process reuse, protection, new observation or Stop. Verify
   expiry during resolution and final native validation, with zero AXPress
   effects after the two-second reference deadline. Test
   protected OCR at 1x/2x and negative origins, ambiguity, queries, languages and
   opaque AX. Require absent password metadata/children and no output on failed
   secure classification. Synthetic Vault-filled plain-text canaries must be
   absent from pixels, AX/OCR, exact repair and every viewer. Unknown provenance,
   missing field mapping or private exclusions withholds observations and input.
6. If the codec and real routes are ready, run `client-relay-matrix.mjs` before
   fatal death. Its 54 fixed cases cover Web/local-TUI/remote-TUI, real Codex,
   Claude/OpenCode, actual VideoToolbox versus PNG, Retina/negative origins,
   loss/reconnect, lock/sleep/wake, permission revoke, crash and signed upgrade.
   Compare exact pixels after settled repair. Keep functional and performance
   verdicts separate. A mocked callback, software fallback in a requested video
   case or missing real provider/route cannot pass. See the M5 checkpoint for
   the callback receipts and metrics contract.
7. Run M3 fatal press/death/reset last. The approved hook observes the actual
   public-fixture key-down during the pending Action and kills only the exact
   paired helper. Require `owner_reset_required` and Stop/Start denial. No later
   input runs until Miguel physically releases and explicitly restarts. Restart
   alone is not release proof. Preserve this failure/reset receipt. Post-down
   programmatic focus loss, foreign or same-process cover, changed geometry,
   hit mismatch and changed key focus must also refuse cleanup and retain
   `owner_reset_required`. Those destructive variants belong at this final gate,
   never before another unattended input. Keep any unrun variant BLOCKED; do
   not chain it after an unresolved press or manufacture a release receipt.

On any functional or cleanup failure, stop further mutations, retain evidence,
revoke the runner's seat if release can be proved, and close only verified owned
processes/windows. Inventory exact cleanup targets. Never recursively remove
credentials, durable homes, shared infrastructure, retained bundles or evidence.

## Verdicts

Keep historical M0 aggregate FAIL and prior accepted M0 click/scroll/capture and
M1 receipts unchanged until exact new-build proofs exist. M2-M5 stay BLOCKED on
missing prerequisites. Local protocol 497, relay peer 106, frozen display #950,
existing CXD1/crypto/events and Cloud #325 remain fixed. No wire allocation,
push, CI, provider run, hosted acceptance or new grant occurred in r14.

The earlier r6/r8/r9/r10 recipes are historical evidence, preserved in Git and
in the lane's `OWNER_RECHECK_R10_HISTORY.md`. They are not additional owner
sessions and must not be chained into this matching-build gate.
