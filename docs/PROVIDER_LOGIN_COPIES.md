# Provider login copies across machines

Status: **decided 2026-10-16** (Miguel). Fix 1 and Fix 2 are **implemented** by lane `credfix` (local protocol 541 / relay peer 149; branch `credfix/r1`). Validation rows CRED-01..04 (area `apps-provider-logins`): kernel/TUI/web behavior is covered by focused tests and a simulated-provider drill through the real kernels, TUI, hosted web (DPR 1 and 2) and relay; the real-Codex cells need a live disposable login and stay open (see "Validation").

This is the decision record for how Chariox moves Codex, Claude and OpenCode subscription logins between machines. Read it before proposing a different shape; the alternatives below were already considered and rejected. Mechanics of profiles, roots and transfer paths live in [PROVIDER_ACCOUNTS.md](PROVIDER_ACCOUNTS.md).

## Decision

Keep the existing shape and add two fixes.

**Shape (unchanged since 2026-10-07).** When a kernel's work is transferred to another Chariox machine, Chariox copies the selected provider logins, records which machine holds which copy, and warns the user once that a copy can later be logged out. When a copy is logged out, Chariox asks for a new sign-in **on that machine only**. No sign-in is needed at transfer time. There is no central refresher and no credential sync between kernels; each kernel stays independent.

**Fix 1: never revoke from a copy.** Before any Chariox-initiated re-login of a profile whose stored credential is a copy, or has tracked copies elsewhere, Chariox deletes the stored credential locally **without revoking it**, then starts the provider's own sign-in flow through the usual in-app link. Signing out on a machine that holds a copy removes the local credential only; revocation happens only when the user explicitly ends the session everywhere.

**Fix 2: detect a dead copy and say so.** When a run or account check fails because the copy's refresh token is no longer accepted, the kernel marks that profile on that machine as needing sign-in, ends the run definitively with the reason and the recovery action, and shows "Sign in again on <machine>" with the device/OAuth link in TUI and web. Healthy accounts show no status. Other machines and profiles are untouched.

## Why copies break, and why the fixes are enough

- Codex (ChatGPT login) and Claude (claude.ai login) use rotating refresh tokens. Every refresh returns a new refresh token and retires the old one; a retired token is still accepted for roughly an hour, then rejected (`refresh_token_reused`, `refresh_token_invalidated`, `refresh_token_expired` on Codex).
- Codex refreshes when the access token is about to expire or `last_refresh` is older than 8 days, and writes the new tokens back to its `auth.json`.
- So two machines holding the same copy both work until one of them refreshes. The other copy then fails at its next refresh. That machine needs one sign-in, after which it has its own independent token chain for good. Short-lived machines usually finish before any refresh happens.
- Since Codex PR #21747, `codex login` first **revokes** the refresh token stored on that machine, and since PR #17825 `codex logout` revokes it too. Revoking a token from a shared chain logs out every machine holding that chain. This, not the copy itself, caused the 2026-10-16 03:16Z outage: a device sign-in on the Mac reviewer's Codex home revoked the chain the b3 lanes were using. Fix 1 removes exactly this failure mode.
- Cost of the shape with both fixes: at most one sign-in per long-lived machine per provider, only when a copy actually dies, instead of one per transfer.

## Implementation (lane `credfix`, protocol 541 / peer 149)

- **What counts as shared.** A profile is shared when this machine received it as a copy (`credential_copy_notices`, set by the worker/slice install and the managed-context import), when it handed copies out (`materializations` for workers and slices, `credential_copies_issued` for managed-context exports), or when it is a materialized replica. Sticky for the life of the profile: after a fresh sign-in on a former copy the chain is independent, but Chariox still never revokes it implicitly (use `--everywhere`).
- **Fix 1.** `release_shared_credential` (`runtime/provider_auth_control.rs`) runs at the start of every Codex sign-in and every Claude terminal sign-in (not the vaulted `setup_token` enrollment) and at every sign-out. It deletes the profile's credential locally (`auth.json`; `.credentials.json` plus the profile-scoped Keychain item on macOS) and invalidates the Codex account endpoint, which keeps the removed login in memory. Sign-out of a shared profile stops there and answers `ProviderLoggedOut { local_only: true }`. `LogoutProvider { end_everywhere: true }` (`/provider logout <provider> <account> --everywhere`) runs the provider logout, which revokes.
- **Fix 2.** The classifier now recognizes Codex's prose refusals (`...because your refresh token was already used / has expired / was revoked. Please log out and sign in again.`). The recovery path marks the profile `expired` on this machine, raises the existing `provider_login` interaction titled `Sign in again on <machine>`, and, if the user cancels or the sign-in fails, ends the prompt with the reason and `sign in again on <machine>`. A Codex account check that reports "logged out" while a refresh-token login is still stored is also reported as `expired`.
- **Exact wording.** Dead copy: title `Sign in again on <machine>` (machine alias, else id), message `<Provider> on <machine>: <reason>. Other machines are not affected. The new sign-in stays on <machine>.` with choices `Sign in` / `Cancel`; the device/OAuth link popup keeps the same title. Reasons (credential-free, from the provider's refusal): `another machine renewed this login first`, `the saved login expired`, `the login was signed out elsewhere`, `the provider no longer accepts the saved login`. If the sign-in is cancelled or fails the prompt ends with `<Provider> sign-in on <machine> did not complete (<reason>); sign in again on <machine>, then retry.` Sign-out of a shared login: TUI `<account> logged out on this machine only; other machines keep their copy (use --everywhere to end it on all)`, web notice `Signed out on this machine only. Other machines keep their copy of this login.` An expired profile's web button reads `Sign in again`.
- **Error mapping.** The official Codex harness reports a dead refresh as `workspace routing discovery unauthorized (401)` (measured against a revoked real login: the log line is `refresh_token_invalidated` / "Your session has ended. Please log in again."); the classifier matches that, the OAuth codes, and Codex's prose refusals. Claude: `API Error: 401` / authentication-failed StopFailure, as before. Home workers and slices never receive Claude refresh credentials (the vaulted setup-token is used at launch), so Claude's copy rules concern new managed Machines and local profiles.
- **Measured provider behavior** (Codex 0.159.3, Claude Code 2.1.296, each run against synthetic credentials behind a logging TLS proxy that records method and path only): `codex login --device-auth` sends `POST auth.openai.com/oauth/revoke` before the device-code request; `codex logout` and app-server `account/logout` send it; the app-server `account/login/start` that the in-app link uses does not revoke at this version, but a newer version may, so the delete is unconditional. `claude auth logout` sends `POST platform.claude.com/v1/oauth/token/revoke`; `claude auth login` does not revoke at start. Claude is treated exactly like Codex.

## Rejected alternatives

| Alternative | Why rejected |
|---|---|
| New machine signs in to every provider at each transfer | Very annoying: every transfer needs one sign-in per provider (Miguel, 2026-10-16). |
| Move the login to the new machine, origin signs in again | Same per-transfer sign-in, plus the origin's running agents must be stopped so they don't refresh the moved chain. |
| One refresher (origin kernel or Cloud) handing out access tokens | Couples kernels to each other, and Chariox must not become the provider credential store (AGENTS.md). |
| Codex host-supplied tokens (`account/login/start` with `chatgptAuthTokens`) for slices | OpenAI documents it as experimental, for host apps that already own the ChatGPT auth lifecycle. |

## What each path does today (main @49a65fbdd)

| Path | Codex | Claude | OpenCode |
|---|---|---|---|
| New managed Machine (managed-context export at creation) | copies `auth.json` | copies the refreshable login (`.credentials.json`, or the profile-scoped macOS Keychain item); rejects non-refreshable credentials | copies `data/opencode/auth.json` + portable config |
| Trusted home-worker / home-managed slice (`ensure_remote_provider_account`) | installs a copy once, then the target owns it | refresh credentials never cross; the vaulted **setup-token** is delivered at launch | installs a copy once |
| Local kernel | the profile's own login | own login, or the vaulted setup-token when enrolled (injected for every launch mode) | own login |

Code: `apps/kernel/src/account_profile.rs` (managed-context export, `discard_nonportable_claude_credentials`, Keychain lookup), `apps/kernel/src/app/remote_lease/provider_account.rs` (worker/slice installation, Claude refusal), `apps/kernel/src/provider/account_credential.rs` (vaulted credential handles).

Notes:
- A Claude setup-token is valid for about a year, does not rotate, and is created through Anthropic's own flow. Copies of it never break each other. It only allows model requests (no Remote Control or claude.ai connectors). Using it for managed Machines too would remove Claude re-logins entirely; that is an optional later simplification, not part of this decision.
- OpenCode API keys do not rotate; copying them is safe.

## Policy position

Two separate questions: whether copies keep working (technical, above) and whether we are allowed to use them (policy).

- **The user's own machines** (local kernels, copies between machines the user runs): allowed. OpenAI documents copying `auth.json` to a trusted machine, with one technical rule: don't share one file across machines at the same time. Anthropic allows a user to sign in to the unmodified Claude Code with their own subscription.
- **Machines Chariox hosts** (managed Machines): to confirm with both providers **before managed Machines go public**. OpenAI's app-server documentation says app-server authentication was never permitted for commercial or hosted services (local and open-source apps may keep using it); their route for hosted products is the "Sign in with ChatGPT" partner program. Anthropic allows hosting the unmodified Claude Code under its Commercial Terms when users sign in with their own subscription, and separately says developers may not collect, store or intermediate claude.ai credentials. Miguel's position: comparable products from real companies do this, so it is expected to be fine; confirm before going public.
- How comparable products handle remote machines (research, 2026-10-16): most avoid user subscriptions on remote machines (their own model access or API keys: Cursor, Factory, Warp, GitHub Agent HQ); local tools reuse the local login (Conductor, Claude Squad, Vibe Kanban); some copy the login file like us (Nimbalyst, Railway for Codex, Domino, whose docs note each refresh invalidates the other copies); some run one central refresher (Terragon, OpenHands, Docker Sandboxes, exe.dev); dev-environment products sign in per machine (Codespaces, DevPod). Railway and Domino use a Claude setup-token for remote machines because the normal login rotates. None asks for a sign-in to every provider on each transfer.

## Validation (CRED rows)

- **CRED-01** dead-copy detection: two homes hold a copy of a disposable Codex login; home A refreshes; after the reuse window home B shows "Sign in again on B" (TUI + web DPR1/DPR2) and its run ends definitively; A keeps working.
- **CRED-02** re-login does not hurt other holders: B's re-login deletes the stored copy without any revoke call (traffic/log capture), the device sign-in completes, and A still refreshes and runs afterwards. The device approval needs Miguel.
- **CRED-03** sign-out on a copy removes it locally only; A unaffected.
- **CRED-04** Claude: same detection and delete-first re-login (fixture level; a real cell needs a disposable Claude login).

Evidence status (2026-10-16): the disposable Codex login on b3 was found revoked, so the real-account cells are BLOCKED until a live disposable login exists (a device approval by the owner). Drills run through the real kernels, relay, hosted web and TUI against a simulated OpenAI side that models rotation, the reuse window and chain revocation; they show the user-visible flows and the revoke-from-copy damage on main, and they never count as acceptance.

Drills use disposable logins only. Never force a refresh, log out, revoke or re-login a shared login.

## Operating rule for our own fleet

To give a machine or consumer its own login: move the copied credential aside (never revoke), run the provider's device sign-in in that home, verify with one real turn on **both** the new home and every other holder, then delete the moved-aside copy. Done this way for the Mac reviewer on 2026-10-16 06:52Z.

## Sources

- Codex auth and CI guidance: https://learn.chatgpt.com/docs/auth, https://learn.chatgpt.com/docs/auth/ci-cd-auth
- Codex app-server (hosted-use note, `chatgptAuthTokens`): https://learn.chatgpt.com/docs/app-server
- Codex revoke-on-login/logout: https://github.com/openai/codex/pull/21747, https://github.com/openai/codex/pull/17825; refresh-token reuse window: https://github.com/openai/codex/issues/10332
- Sign in with ChatGPT for third-party apps: https://developers.openai.com/siwc/token-sharing-open-source
- Claude Code authentication and setup-token: https://code.claude.com/docs/en/authentication
- Claude Code legal and compliance (hosting, credential use): https://code.claude.com/docs/en/legal-and-compliance
