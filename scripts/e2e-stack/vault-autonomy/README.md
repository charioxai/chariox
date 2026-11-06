# Vault autonomy drills — MP-08 / MP-09 / MP-10 / MP-11

These are real-service drill helpers, not kernel behavior. Keep `CHARIOX_HOME`,
`TMPDIR`, setup state, screenshots and logs outside the checkout. Run the real
new UI, matching kernel and hosted relay with an official provider. A helper
unit test alone does not establish AU acceptance.

## AU-04: dynv6

`dynv6.py --state /absolute/lane/private/account.json signup` takes the newly
generated disposable account's `email` and `password` on stdin. It writes a
mode-0600 cleanup record **before** sending the signup request. The state
directory is mode 0700. Use a persistent lane directory outside the disposable
kernel fixture. Never replace an existing cleanup record with a new signup.

The service can sign a new account in immediately and still require email
confirmation. An authenticated `/zones/new` page is signup success; it need
not contain the old “confirmation link has been sent” message. The classifier
requires an observed account control, and setup verifies a new login. Follow
the real confirmation email and verify the account's confirmed state through
`status` before DNS publication. The observed public confirmation page has a
human-verification step: GET alone does not activate the account. Never solve
or bypass that challenge. An unconfirmed account can create API records while
public DNS still returns NXDOMAIN; that is not propagation acceptance.

Prepare disposable setup with `prepare-dns.py --state-dir /absolute/lane/private`.
If human activation is unavailable, record BLOCKED and clean up; do not create
more accounts hoping the gate disappears. The Gmail browser is not used by
this helper.

Copy the helper into the drill workspace. The official agent must invoke
`capture` only through `chariox.vault.capture_command`, with an explicit
`command_environment` binding for `DYNV6_TOKEN` at `https://dynv6.com`. Its
stdout contains the real credential and must go directly to Vault. The helper
uses the service's actual login/Keys forms and verifies the candidate against
the [documented REST API](https://dynv6.com/docs/apis).

The agent then runs `red` to establish the unauthenticated refusal and calls
`chariox.vault.use_env` with the same state path and `create <zone> <proof>`.
Use a unique disposable `*.dynv6.net` zone with a hostname label of at most
20 characters. Schedule one real one-shot timer,
yield, and on its wake run `probe` through the same binding. Retain the actual
event identity, one fired occurrence, public TXT response and screenshots at
DPR 1 and 2. Do not count a final answer marker as proof of a timer firing.

Cleanup runs after every attempted signup, including uncertain responses and
failed capture/create/probe. Delete the created zone through `delete-zone`,
then run `delete-account` through the normal private setup client. Account
cleanup uses the site's observed `credential-settings` account URL, current
password and CSRF-protected DELETE, and requires the successful cancellation
response plus refusal of a fresh login. Persist the acknowledgement before
following the returned location so an interrupted verification can reconcile
without deleting twice. Only then remove private setup. Delete the disposable confirmation
mailbox last. If cleanup fails, preserve setup and report the exact failure;
fixture teardown must not discard recovery access. Record account, zone and
mailbox cleanup separately from acceptance.

`dns-live.mjs` drives the real built new UI and official Codex through the
hosted relay. Set absolute `VFLAG_CLOUD_REPO`, `VFLAG_BIN`,
`VFLAG_BUILD_MANIFEST`, `VFLAG_PROFILE_HOME`, `VFLAG_SETUP` and
`VFLAG_EVIDENCE`; also set `VFLAG_ORIGIN`, `VFLAG_OWNER_EMAIL`,
`VFLAG_CLOUD_HEAD` and `VFLAG_DPR` (1 or 2). The Cloud checkout supplies the
existing `external-access-kernel-fixture.mjs` setup helper and Playwright.
Pass only your own validation stack's generated access key on fd 3. Use
`boundrun.sh` and a disk-backed `TMPDIR`; `SHAPE_PROXY` accepts the lane's
opaque CONNECT shaper. No provider account credential is copied: the kernel
links the existing product-created profile through `LinkProviderAccountProfile`.
The activation check blocks before capture/publication, then performs cleanup.
The retained r2 proof is partial and blocked at activation, not a PASS for this
complete runner.

Supplementary regression checks:

```sh
python3 -B -m unittest discover -s scripts/e2e-stack/vault-autonomy -v
```

## AU-01 / AU-02 / AU-11: flagship (`flagship-live.mjs`)

The real built new UI, hosted WSS relay and official Codex agent: owner creates, reveals (hold) and replaces a Vault
entry, an ordinary GitHub fill, exact-origin refusal, a real Mail.tm token captured by `capture_command` and used by
`use_env`, rotation and revocation. `VFLAG_BINDING=bound VFLAG_MAIL=1 VFLAG_EXTRA=1`; the stack's generated access key
enters on fd 3 and `VFLAG_OWNER_EMAIL` names its fixture identity.

## AU-15 / AU-06 / AU-11: Neon (`neon-live.mjs`)

Neon's claimable service (`https://neon.com/auth.md`, anonymous flow, no account) is a public auth.md service that
needs no human step. Run with the same environment as the flagship driver. Turn 1: the agent calls
`chariox.agent_auth.register` with `return_fields ["/project/id","/project/expires_at"]` (Neon scopes its API to
the project the registration created, so the id is needed) and makes one 200 call. Turn 2: it mints a second token with
`capture_command`, fetches the project's `database_url` with `use_env` into a file, captures that file into the Vault
with a file binding, writes `.env` from the handle, and a small app queries the real database. The harness repeats
the query itself with the `.env` value and scans transcripts and artifacts for the URL and password. Last turn:
the agent records the project in the resource register and revokes it (HTTP 204, then 401). Neon deletes unclaimed
projects after 72 hours; the drill deletes its own.

## AU-07: OAuth round trip (`oauth-capture-live.mjs`)

Same setup as the AU-10 Gitea drill (`TCAP_*`). The owner stores the Gitea password; the agent signs in, creates an
OAuth2 application whose redirect URI is the harness-run relying party (`oauth-relying-party.mjs`, which reads
`CLIENT_ID`/`CLIENT_SECRET` from the workspace `.env` on every request and exchanges the code server side),
captures the one-time client secret with `capture_browser` bound to that `.env` file, writes `.env` with
`write_file`, then drives the login and clicks Authorize. The driver checks the relying party's exchange and the
user it read, Gitea's own consent grant and application rows, the file binding and the absence scans.

## AU-05 / AU-06: account-less Cloudflare Workers (`deploy-live.mjs`)

Cloudflare publishes account-less ("temporary") deployments: `npx wrangler deploy --temporary` and
`npx wrangler secret put NAME --temporary` need no account and no human; Cloudflare deletes the temporary account
after 60 minutes unless claimed (nobody claims it). Same environment as the flagship driver. The owner stores a
random secret for the Cloudflare command's stdin; the agent deploys a Worker that returns the SHA-256 of
`env.APP_SECRET` and sets the secret with `use_env` stdin, and the harness fetches the live URL and compares the
hash with its own value. The agent then mints a Neon token (`capture_command`), fetches the database URL, captures
it twice (a `.env` file binding and a Cloudflare stdin binding), writes `.env` with `write_file`, and deploys a Worker
that queries Neon through `@neondatabase/serverless` with the stdin-bound secret; the harness fetches that URL and
repeats the query itself. Run every wrangler command with one `XDG_CONFIG_HOME`, or the Worker and its secret land in
different temporary accounts.

## AU-16: scope of the existing substitute mechanism

At source `68c0f04848`, AU-16 remains a product feature gap. It is not blocked
on provider credentials. The row requires a refused **step** to complete on a
second provider without owner intervention, with the refusal recorded.

The current contracts do different jobs:

| Contract | Entry point | Behavior |
| --- | --- | --- |
| Failed provider turn | `runtime/state/provider_substitute_runtime.rs::rerun_failed_turn_on_substitute` | Replays the active prompt on the next saved substitute. Requires an error outcome and excludes cancellation, external/native and remote turns. |
| Completed answer | `provider/turn_outcome.rs::TurnOutcome::settled` | A completed turn stays completed, regardless of its answer text. There is no typed model-refusal outcome. |
| Browser handoff | `runtime/state/owner_handoff_runtime.rs::kernel_browser_handoff` | Validates an observed browser target and creates a durable owner interaction and handoff obligation, including for `model_refusal`. It does not dispatch a substitute. |

The [substitutes contract](../../../docs/AGENT_SUBSTITUTES_PLAN.md) explicitly
does not treat completed answer text as a provider error. Reclassifying any
answer containing “cannot” would be incorrect. Replaying the whole goal after
a refusal can repeat already completed account, DNS, deployment or payment
operations. An owner handoff's browser node and change description are not a
durable, idempotent resume point for that goal.

S9 therefore needs an explicit step-level continuation contract: identify the
unfinished operation and completed effects, distinguish an eligible execution
failure from a provider safety refusal or a site/human-verification restriction,
settle any outstanding tool/owner interaction, persist the bounded attempted
substitutes and result, and recover without replaying effects after restart.
It should reuse normal provider launch, existing grants, task obligations and
substitute profiles. It must not route around provider safety refusals or
human-verification requirements. The retired mandate proposal is not an
authority path to revive.

That contract and its real-provider, interrupted/restarted, side-effecting
drill exceed this signup-harness repair. No automatic refusal-text matching,
handoff bypass, new mandate or acceptance PASS is introduced here. S9 remains
FAIL until that separate behavior and live proof exist.

## AU-10: TOTP enrolment by capture (Gitea)

`totp-capture-live.mjs` runs an official Codex agent against a real Gitea
(`gitea-service.mjs`, downloaded outside the repository and run as the
unprivileged `ubuntu` user under `/home/ubuntu/vgaps-*`). The owner stores the
account password in the Vault for the one Gitea origin; the agent signs in with
it, then enrols two-factor by calling `chariox.vault.capture_browser` with
`totp: true`, pointing at the setup key text (`TCAP_MODE=key`) or at the QR
image with `read: "qr"` (`TCAP_MODE=qr`). It fills the passcode field with the
returned handle, signs out, and signs in twice more, the second time after one
real timer wake so Gitea sees a different period. The driver checks that the
captured entry is a TOTP seed bound to the one origin, that Gitea itself now
requires two-factor and stored a different passcode at each login, that no
approval interaction appeared, and that no seed or code is in the provider
transcripts or kernel artifacts. Run it under Bun (the Cloud fixture needs
`argon2Sync`) through `agents/boundrun.sh`, with absolute `TCAP_*` paths:
`TCAP_CLOUD_REPO` (a Cloud checkout with `node_modules`), `TCAP_BIN` (a
directory with `chariox-kernel` and `chariox-relay`), `TCAP_PROFILE_HOME` (a
product-linked Codex profile copy), `TCAP_GITEA`, `TCAP_ORIGIN` (an owned
`pr<N>-vgaps*` hosted stack), `TCAP_EVIDENCE`, `TCAP_BUILD_MANIFEST`,
`TCAP_CLOUD_HEAD`, `TCAP_DPR` and `TCAP_MODE`.

## Agent-driven account rows

`room-driver/` runs AU-03, AU-04, AU-08 and AU-16 with a real Room agent and a person for the steps only a person can do (see its README).

## Verification cards and owner browser recovery (MP-08 / MP-09 / MP-10 / MP-11)

`challenge-hosted-live.mjs` uses an owned loopback service that sends no email.
Use Node24 (the hosted fixture requires argon2Sync) and run through the new
hosted UI with an official linked Codex profile. Supply
`VFLAG_ORIGIN`, `VFLAG_EVIDENCE`, `VFLAG_BIN`, `VFLAG_CODEX_PROFILE` and an external
`TMPDIR`; use `VFLAG_ENCODING=json` and `urlencoded` in separate runs. Desktop
DPR1 and DPR2 are the default cells. The drill keeps a second owner viewer open
at the other density, requests one code in the popup, submits a wrong code,
checks the rejection stays visible, then opens the same tab and submits the
correct code. Both clients must remove the same card, and the agent must
capture the temporary token, make an authenticated request, and remove its
capture without Resume. Screenshots and status-only events stay under the
evidence path. Temporary token fields are password fields in screenshots.

The explicitly authorized `pr0-gmailprep` run accepts
`VFLAG_EXISTING_KERNEL_ROOT=/root/.chariox/dev/vault-drill-kernel`. It connects to
the running kernel through owner IPC, uses the normal owner sign-in form, and
creates a separate disposable workspace/session. It never starts or stops that
kernel, changes its Vault configuration, requests a real Vercel code or closes
its other sessions. The Vault must already be unlocked through the owner UI.
Use product hot reload for upgrades to preserve an existing unlock. This mode
uses the existing linked profile (`VFLAG_PROFILE_ID`, default
`vflag-r4-drill-owdpqv4h`) and registers a new workspace owned by the real kernel
service account. A pending browser-access request uses
`chariox.events.blocked` with `owner_action: "browser_access"`; its owner card
allows browser access and continues the original task in one click. Ordinary
blocked reasons cannot create that grant decision.

Set `VFLAG_RECOVER_BROWSER=1` to revoke access after the first card, approve
Allow browser in place, and require a fresh card on the same tab before the
wrong-code/retry loop. `VFLAG_EXPECT=grant-red` records the old stuck path;
`encoding-red` records the old accepted-but-still-waiting URLencoded path.
Green checks include plain card text, no automatic card expiry, and at least an
hour on an active person wait. This focused drill supplements AU-03; it does not
claim a real Vercel token was created.

## Single-use owner unlock (MP-08 / MP-09 / MP-10 / MP-11)

`once-unlock-hosted-live.mjs` uses the real hosted Web/WSS path and an official
Codex agent at DPR1 and DPR2. The owner selects "Just this once" in the Vault
window, then the agent generates one credential for `https://github.com`.
The Vault must close, and a new generation request ID must ask for another
unlock. Unlocking from the Vault window while that request waits must resume
it, withdraw its card, and close the Vault again after generation.
Green also observes the real GitHub login page, then fills its password and
plain username fields through the two agent fill tools without submitting.
Each fill receives its own owner-window "once" unlock and must close the Vault.

Use Node24 through the lane's bounded runner. Supply `VFLAG_ORIGIN` (an owned
`pr<N>-vflag*` stack), `VFLAG_EVIDENCE`, `VFLAG_BIN` (private exact-source kernel
and relay), `VFLAG_CODEX_PROFILE` (a product-linked profile), and an external
`TMPDIR`. `VFLAG_CLOUD_ROOT` selects the Cloud checkout providing the existing
hosted fixture helpers. `VFLAG_EXPECT=red` records the pre-fix bypass, including
the second credential created without another unlock; green is the default.
Keep binary hashes and source revisions beside the results. The drill closes
its own clients, Room, kernel, relay and provider processes. It exercises the
Vault operation rather than creating a GitHub account, and does not close the
broader provider or ordinary/managed parity acceptance matrix.
