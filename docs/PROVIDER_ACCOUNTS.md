# Provider accounts

Chariox supports multiple named account profiles for Codex, Claude, and OpenCode. The kernel owns profile metadata, selection, status/usage projection, and orchestration. Provider CLIs remain the credential-format and token-refresh authority.

How login copies behave across machines (re-login without revoking, dead-copy detection, policy position, rejected alternatives) is decided in [PROVIDER_LOGIN_COPIES.md](PROVIDER_LOGIN_COPIES.md).

## User workflow

Open **Provider Accounts** in either Room list to list, create, link, rename, set a default, refresh, log in, log out, remove, or explicitly delete a managed profile. The launch form selects Provider, Account, Model, and Variant. New agents inherit the selected account unless another profile is chosen.

Changing an existing agent's account uses the same bounded context handoff used for provider/model changes. The active provider run ends, incompatible provider resume state is cleared, and a fresh run starts under the selected profile. Credentials are never hot-mutated in a running provider process.

The configured Cloud owner and local TUI share the home kernel's account registry. Collaborators retain separate namespaces and cannot list, use, or receive the host owner's profiles.

New managed Machines default to every authenticated, transferable profile discovered by the selected source kernel. The user may exclude profiles or disable account transfer. Before asking Cloud to create the Machine, the source kernel exports each selected profile into its provider-native portable credential shape. Claude credentials are accepted only when they contain a non-empty refresh token; on macOS the exact profile-scoped Keychain item is used when the profile directory has no portable credential file, with the legacy unscoped item allowed only for the default profile. A missing or non-portable credential stops the launch before compute is rented. The managed-context plan still records an explicit canonical profile list; Cloud never receives credential contents.

A login held in the Vault (a Claude setup token) is carried too, with no step on the new Machine. The source kernel reads the value through `RuntimeSecretService` under the profile's deterministic handle and seals it for the new Machine's relay key, bound to the context, both kernel ids, the provider and the profile. The package, its digest and the receipt hold ciphertext and ids only. The new Machine opens the seal, runs the provider's own check, writes the value into its own Vault (created from the source's sealed key seed, local 973) under the same handle, and only then registers the profile authenticated. A profile already on the Machine under that id refuses the transfer and the accounts installed before it are rolled back. Preflight accepts a Vault login while the source Vault is a Chariox Vault; a locked Vault shows the existing unlock card in the launching session (a launch from an agent included) and a launch from a client without a session gets the typed `vault_locked` refusal that the launch dialog answers with its unlock card. Under "ask every time" the unlock preflight took is kept for the export (one use, at most 10 minutes) and closes the Vault when the export has read the login; when it ran out, the export asks again with the same unlock card. `context_transfer` is `unavailable` only for a login whose Vault cannot be sealed for a new Machine (a Vault that is not a Chariox Vault).

## Provider roots

- Codex: every profile has a distinct `CODEX_HOME`. Managed profiles force `cli_auth_credentials_store = "file"`; `auth.json`, app-server processes, catalogs, usage, login, and logout are profile-scoped.
- Claude: managed and directory-linked profiles use an explicit `CLAUDE_CONFIG_DIR`. An effective native default registered without that variable preserves its absence. These are different credential scopes on macOS, even when the explicit directory is `$HOME/.claude`. Chariox invokes the official `claude auth login`, `logout`, and `status` commands in the selected scope.
- OpenCode: every profile has distinct data, config, state, cache, and `OPENCODE_CONFIG_DIR` roots. A profile may contain multiple upstream connections. Upstream usage is a capability matrix: local stats and supported native seams are projected, while unknown billing providers report unavailable rather than guessing or reading secrets for third-party APIs.

Existing effective default roots migrate once into the durable registry with stable profile IDs and public labels. `default` resolves the currently selected default; it is not a replacement for a stored profile ID. Static provider-profile configuration is not a second source of truth.

Linking a provider directory already registered under the same owner and provider returns an error identifying the existing profile. Claude directory links also compare the provider's OAuth account UUID when it is available. A second directory for the same account returns the existing name. Native account transfer carries this non-secret identifier separately from the provider data.

An upgrade groups saved Claude profiles with the same known account UUID into one account and one Usage row. The default profile's explicit name wins when it has one. Otherwise the first explicitly named registration supplies the name. The registry records the names that were merged. Existing profiles keep their execution directories and lookups so agents can still resume in their original Claude session. A profile whose login exposes no account UUID is kept separate.

Profiles created without a label follow the authenticated account email's local part. A rename makes the label user-owned, even when the chosen label equals the current automatic name. Colliding automatic labels receive a numeric suffix; reserved or invalid email names leave the last label unchanged. Legacy profiles without naming metadata treat provider-number aliases and names matching the last observed email local part as automatic; other labels are preserved.

### Claude native login and legacy profiles

A successful native `claude auth status` does not prove that a directory-linked Chariox profile is signed in. First compare the selected profile's credential scope with the native invocation. Preserve the real macOS HOME. Do not copy credentials, log out, or start another login merely because the two status results differ.

Legacy registries did not record whether `$HOME/.claude` was an ambient default or an explicit `CLAUDE_CONFIG_DIR`. Migration preserves explicit scope for those ambiguous records rather than silently switching accounts. Refreshing status or choosing that profile as the default does not change its scope. Linking `$HOME/.claude` also creates an explicit directory scope, not an ambient-native account.

Use `/provider accounts import-native <provider>` to register the kernel host's current native scope explicitly. Import is idempotent for a scope already registered. When the native scope differs from an ambiguous legacy profile, it creates a separate stable profile and does not replace the old registration, change the default, start login, or copy provider files. Select or mark the imported profile as default only after its native status is refreshed. Do not edit a running kernel's registry file or remove/recreate profiles that existing agents depend on. Registration removal preserves provider files but does not preserve references to the removed profile ID.

## Logins already on the machine

Chariox does not ask for a login the machine already has, and it does not keep a second copy of one. When the kernel starts, and again when a client opens the Room list (at most once a minute), it looks for the installed `codex`, `claude` and `opencode` CLIs (on `PATH` and in the usual user install directories such as `~/.local/bin` and `~/.opencode/bin`) and asks each CLI about the machine-level login of the default profile. A valid login marks the profile authenticated with its identity, the model catalog is rebuilt and warmed, and every client is told, so the first model picker already lists models.

Automatic catalog warm-up skips default-origin profiles until they are authenticated. An explicit provider or account catalog request still starts its server on demand. Codex discovery leaves an empty native default unprobed until its native login file exists, so an idle kernel does not keep an unauthenticated app-server alive. A later discovery can find a new CLI login.

The default profile points at the provider-native location in place. A second kernel of the same OS user therefore finds the same logins at its first start with no step for the user. Two further kinds of provider-native location are registered the same way, once per kernel and only when the CLI reports a valid login for an account no profile has yet:

- roots a kernel of this OS user linked with `LinkProviderAccountProfile` outside its own home (the paths, never credentials, are listed in `~/.config/chariox/provider-locations.json`);
- other Claude Code configuration directories next to `~/.claude` that hold a login (`.credentials.json`, or on macOS the `Claude Code-credentials-<hash>` Keychain item, whose existence is checked without reading it).

A location the kernel has considered is remembered, so removing a discovered profile is not undone by the next pass. An account that exists only inside another kernel's private home is not linked; it stays an explicit copy.

Any change to who is signed in, however it happened, invalidates the catalog and raises a client catalog change, so clients refetch accounts and models without a reload.

## Workers and slices

The home kernel owns the agent and selects the account profile used for its initial placement. When an agent is assigned to a trusted home-worker or home-managed slice, only that selected profile is installed through the existing encrypted kernel-to-worker channel. Separate profiles use separate roots. Claude uses its provider-native portable `.credentials.json` shape after refreshability validation. Cloud and the relay receive only opaque encrypted packets and safe installation status.

Installation is denied before launch when the existing trust/ownership policy does not authorize credential transfer. After the target accepts the profile, that kernel owns its files and provider-native state exactly like a locally created profile. Later agent launches reuse it without exporting or retransmitting source credentials. Source-side login, logout, or account changes do not mutate the installed target profile. Replacing credentials is an explicit operation on the target kernel. A later execution lease from that source kernel for the same account runs on the installed profile; the worker makes no second copy, and the lease names the worker's own profile id.

Each vaulted provider credential has a deterministic, non-secret handle derived from the account owner, provider family, and stable profile ID (`claude`, `claude-p` and `claude-headless` agents of one account share it). Launch preparation resolves that handle through `RuntimeSecretService`. Local launch keeps the resulting value in a redacted, non-serializable, zeroizing environment. A leased launch carries no credential: the worker resolves its own tracked copy of the account (see Leased Claude agents). The value is excluded from provider-run persistence, projections, command history, and debug output. Claude's live runtime retains that protected environment only so a provider child process can restart without returning to macOS credential storage.

After creating the setup token with the official `claude setup-token` command, an interactive Chariox client stores it with `provider setup-token claude <account-profile>`. Input is hidden. Replacement requires `--replace`. For MP-08 / MP-11, the kernel verifies that the profile belongs to the caller's account authority and that the token completes a bounded official Claude model turn with tools disabled, requests a Chariox Vault unlock through `RuntimeInteraction` when needed, and writes the vault value and provider-only credential policy as one operation. The setup token is never stored in the Claude profile directory or macOS Keychain.

For MP-08 / MP-11 (post-merge), `provider setup-token claude <account-profile> --run` runs the official `claude setup-token` inside a kernel-owned PTY on the kernel that owns the profile. The TUI also accepts `/provider setup-token claude <account-profile> --run`; Web Provider Accounts offers **Create setup token** and explicit **Replace setup token** actions on protocol 411 or newer. Existing entries require `--replace` before minting. The request uses the existing `StartProviderLogin` method `setup_token`: a 1,000-column terminal render captures the complete token, the same model-turn authentication verification runs before storage, and the login workflow asks for the Vault passphrase only when storing it.

For MP-08 / MP-11, the existing terminal renderer projects readable provider output, including the real `https://claude.com/cai/oauth/authorize` link, while hiding credential-shaped values and authorization-code echoes. The shared screen receives raw bytes before PTY queues or diagnostic tails; only its redacted text reaches login status. Storage requires one complete `sk-ant-oat01-` candidate with at least 80 body characters, a successful CLI exit, and `verify_claude_setup_token` model-turn verification. Missing, short or multiple candidates, verification failure, timeout, cancellation and CLI failure store nothing. Status, hidden authorization-code input, Vault passphrase entry and cancel use the normal provider login workflow (`provider login-status`, `provider login-input`, `provider login-cancel`); clients and Cloud never receive the resulting token. Vault I/O runs outside the login-store mutex and unlocks only for the storage operation. The manual hidden-input command remains supported.

The fake-CLI kernel drill covers reader chunk boundaries, Vault storage/replacement, hidden-input echo suppression, CLI failure, missing/multiple candidates, cancellation, and scans of client output plus disposable state/log/history. This is source evidence for MP-08 / MP-10 / MP-11, not signed-release or real-provider acceptance. The owner performs the real Claude authorization drill.

Model catalogs are cached by owner, selected profile, and execution location. Remote/slice selections require evidence that the profile was installed at that target; source-profile freshness does not invalidate an independent target profile. Clients never infer availability from labels.

OpenCode account transfer exports `data/opencode/auth.json` and the portable
configuration files `config`, `config.json`, `opencode.json`, `opencode.jsonc`,
`tui.json`, and `tui.jsonc` from the profile's global and custom configuration
directories. It does not traverse the data, state, or configuration trees.
Session databases, prompt history, snapshots, caches, locks, installed
`node_modules`, and capability packages are not account credentials and do not
belong in this transfer. Provider-native configuration continues to refer to
capabilities installed at the execution location; managed-slice capability
transfer uses its separate existing kernel path. Missing optional files are
allowed, but exported roots and files must be regular, non-symlink entries and
the existing 64 MiB total limit still applies. Managed-machine context export
remains the separate credential-only, 16 MiB path.

Agent launch never refreshes an installed OpenCode profile. The target kernel and
OpenCode own its credentials, configuration, history, databases, and later
changes. An explicit account-transfer or credential-update operation may replace
portable account files, but it is not coupled to provider launch.
Kernel startup follows the same rule for publication-bound accounts. The
publication manifest installs a missing profile once; restarting the kernel does
not reapply source credentials or reset the target kernel's default account.

This account transfer is not a history backup or provider-session migration.
Environment and slice saved-state acceptance must validate their own durable
provider state independently.

### A Claude agent on another machine

Row 274. An agent spawned on another machine carries its Claude login only as the Vault setup token, not as the native credential file. This is the launch's own carry and treats a rented machine like any other kernel; account copy to another kernel, a rented machine included, is the separate explicit operation above. When an agent is spawned on another machine (a rented machine included) with a Claude profile, the home kernel decides before anything is created there. A profile held as a Vault setup token spawns with no step and no card: the launch carries the token, and the native credential file beside it is no longer exported. A profile that is a native login only refuses the spawn and raises ONE card on the owner's own computer (per profile, in the spawning agent's Room): **Create login**. Choosing it runs the existing setup-token sign-in on that kernel, the one person step; afterwards the same spawn works with no step on the other machine. A spawn from a client with no agent has no card; the refusal names the same step. The refusal and the card never ask for anything on the other machine, and no credential value appears in them.

## Leased Claude agents

A Claude agent leased to a worker runs from the worker's own tracked copy of the owner's Claude account. No launch packet, prompt request or relaunch carries a token, so no kernel depends on another after the copy is installed.

- The first time the worker needs the account, the home kernel makes one tracked account copy to it, the same `CopyProviderAccountToKernel` operation the owner can start. It needs no card and shows no warning. One operation per account and worker means a retry returns the saved receipt and nothing is sent twice. The home records the copy as a `copy` materialization of the profile; the worker records it as the profile's `transferred_from`. Provider Accounts shows both ends.
- The home names the account by its own profile id. The worker finds its copy by where it came from (the lease's home kernel and that profile id), never by a label, and the leased and backing agent move to the copy's id before the first launch. A launch, a prompt and a process restart on the worker all use the copy's Vault handle, with the home offline.
- Only the owner's own enrolled kernels receive a copy. A worker that is not one gets `provider_account_transfer_target_not_owned`; a worker too old to hold a copy gets the peer-protocol refusal naming the update; a locked Vault gives the existing unlock card, which the held prompt waits on. Each refusal names one step and none asks the person to type a command.
- When the owner has no Claude token for the account (no login, or only a native one), the kernel runs the official `claude setup-token` flow itself, in the existing provider-login card, with the link and the masked code field in every client. The prompt is held while the card is open and goes on by itself when the person finishes the sign-in: the copy follows and the prompt launches, with nothing sent again. If the card is cancelled the prompt ends with `provider_account_transfer_sign_in_required`. A locked home Vault gets the existing Vault unlock card the same way, and the prompt goes on after it is answered.

**Sign-in rule (Miguel 10-08):** the person never runs a command for provider sign-in. The kernel runs the official flow and shows the link/code in every client.

## Usage semantics

An automatic usage stop keeps its account on the existing usage refresh cadence. A newer reading with fresh relevant quota or credit capacity resumes the stopped turn before its scheduled reset; stale or missing readings do not. The schedule prefers the relevant quota meter's absolute reset time, and provider text is used only with a named time zone. Repeating `agent resume` preserves the armed time while usage is exhausted and resumes now when a newer reading confirms room. An accepted new turn clears an unclaimed stop even on the same account. Room clients show an armed stop as “Waiting for usage, resumes at <time>” rather than asking the owner for help. Notes queue during the wait. Sending a queued note now to an idle waiting agent asks the owner to resume it or choose another account; the note stays queued.

Usage meters identify their source, kind, unit, limits/balance where exposed, reset time, freshness, and availability. Missing numbers mean the provider did not expose them; they are not treated as zero. Codex subscription windows and credits use app-server methods. Claude uses provider-native rate-limit observations and an explicit official-CLI `/usage` refresh with tools disabled and session persistence disabled. The refresh accepts only structured results with all required model-activity fields present and zero; missing fields are not assumed to be zero. Chariox does not rewrite Claude's onboarding or trust settings for this probe. OpenCode billing remains best-effort and extensible per upstream provider.

Claude asks a person once whether to trust a folder. Before any Claude launch, the kernel records that answer for the folder Chariox chose for the agent (its own worktree, else the Room's) as `projects[<folder>].hasTrustDialogAccepted` in the launch profile's `.claude.json`, on the machine where Claude runs. The write is atomic and changes nothing else Claude wrote. The launch directory must be that folder; the home directory and a filesystem root are never recorded, and any other folder Claude asks about still reaches a person as a `claude-workspace-review` permission card. A state file the kernel cannot update fails the launch with the reason.

## Security and deletion

Profile paths and credential values are private kernel state and never appear in Room list, Cloud, relay, or protocol projections. Ambient provider API-key variables are scrubbed from managed launches so a named profile cannot silently execute under unrelated environment credentials.

Logout, deregistration, and deletion reject profiles with active runs. Removing a profile keeps provider data. Deleting managed data is a separate operation requiring the exact profile ID and is unavailable for linked/default roots.

## Live validation

`apps/cli/scripts/live-multi-account-drill.mjs` is opt-in and accepts existing profile IDs. It never copies or prints credentials. OpenCode remaining-balance validation is intentionally pending until suitable accounts/upstreams are available; unsupported sources remain visible as unavailable capability states.

MP-08 / MP-11: `claude`, `claude-p`, and `claude-headless` share one account credential identity. Direct token import records authenticated state only after verification and successful Vault storage. Explicit Status and Refresh requests verify the currently stored token with a bounded official model turn. Only a successful check records authentication; the validation timestamp records the latest saved observation, including a definitive rejection; unavailable or inconclusive checks leave it unchanged. Prior native-login observations never stand in for token verification. A locked Vault or missing Claude executable leaves the saved observation unchanged and reports that verification is unavailable. A definitive credential rejection records `Expired`, so a subsequent launch enters the shared login recovery without another verification turn. A token replaced during a check records `Error` and requires verification of the replacement. Inconclusive turns, including transport failures and unexpected result envelopes, preserve the prior observation and return a redacted classification (exit status, HTTP status when reported, turn count and single-turn validation), never provider text. Enrolled Vault tokens are injected for every Claude launch mode on Linux and macOS, even when a previous native login exists. A stored handle alone never proves authentication, and setup-token authentication does not fabricate subscription usage. Automatic pre-launch usage refresh, login completion, managed-context import, publication startup, and publication health never run setup-token verification turns or replace the saved token observation. Publication startup shares the automatic token-observation guard and retains the native-login auth-only check. Publication health reads saved profile state; imported unknown token profiles need an explicit user Refresh. These refresh paths spend verification turns only on enrollment or explicit Status/Refresh. The ordinary first-use recovery for older unverified accounts remains shared.


## Explicit account copy to another kernel

MP-08 / MP-09 / MP-10 / MP-11: `CopyProviderAccountToKernel` copies one authenticated Claude, Codex or OpenCode account to an enrolled same-owner kernel over the encrypted peer channel. An enrolled same-owner kernel includes the owner's own rented Chariox machine: it enrolls with its machine credential instead of a kernel grant, Cloud lists it for account copy (peer list `purpose=provider_account_copy`) and the receiving kernel accepts that enrollment; a rented machine is a kernel like any other and needs no other path. Ordinary owner Copy stays credential-free. Claude uses its provider-only Vault handle. Codex and OpenCode export the current native credential through the credential-only account materialization path (`auth.json`, `data/opencode/auth.json`), as one string of at most 16 KiB; a larger login is refused at the source with `provider_account_copy_credential_too_large`. The target checks a private staging profile with the official CLI before it publishes anything. Codex is stored in the Vault and published as the target's own native auth file. OpenCode lives only in its native store and needs no Vault on either kernel. Later launches use that file, and completed retries never replay the original transfer over a refreshed target.

OpenCode has no login-check command. The target requires a usable credential, `opencode auth list` to accept the staged files, and, whenever the login holds an OpenCode Go key, the one authenticated provider call the kernel knows (the Go usage request): a rejected key refuses the copy, an unreachable provider refuses it as not verified. Other services in the login are checked only locally. The source asks for the `provider_account_copy_opencode_v1` capability first, so a kernel without it refuses with `provider_account_copy_unsupported` before anything is exported.

A refusal names its real cause. A Vault that cannot be read (`..._vault_unreadable`, `..._target_vault_unreadable`) is not a locked Vault (`..._vault_locked`, `..._target_vault_locked`); a stored login that cannot be read back is `..._credential_unreadable` or `..._target_credential_unreadable`; a receiver without the provider's CLI is `..._cli_missing`; and the same operation retried with only `replace` changed is `..._replace_differs`. Any other difference on a retried operation stays `..._conflict`.

A copy works immediately. Copies of one Codex login initially share a refresh lineage, so the target notification advises signing in there once through its own device sign-in. That creates a separate login. If a copy later expires, the Inbox says **Sign in again on <machine>** and starts sign-in on that target kernel. Kernels have no refresh dependency on each other.

On the same kernel, a copy request creates a linked profile sharing the source credential scope. Codex aliases share one catalog helper; a change to the native auth file retires the cached helper. A retained login that the provider cannot use saves an actionable `auth_error`, instead of silently becoming unconfigured. Local link offers are available while relay is disconnected. See [the account-copy drill](drills/PROVIDER_ACCOUNT_COPY.md) for the request, conflict/retry rules and real acceptance requirements.

## Signing in from a Vault entry

An API-key provider sign-in (OpenCode Go and Zen) takes a Vault entry instead of a pasted key. `provider login <provider> [account-profile] --method opencode_go_api_key --vault <credential_id>` starts the sign-in and sends the entry; Web Provider Accounts offers **Use a Vault entry** next to the paste field, listing the owner's entries for the provider's site. The kernel reads the value, types it into the provider's own login terminal and lets `opencode auth login` finish; the value never reaches a model, the status, history, a log or a tool result, and the login screen stays masked.

The provider-to-site map is one function, `provider_enrollment_vault_origin` (OpenCode: `https://opencode.ai`). The entry must be a secret bound to exactly that site for browser or HTTP use, so an entry the owner created for the site and one `chariox.vault.copy_for_use` made for it both work, with no copy step. An entry for another site is refused with the site it must be bound to. No other Vault use widens: the entry is read only for this sign-in.

### Run a private Linux drill kernel

An agent shell already runs inside Bubblewrap. A drill kernel started in that
shell inherits the Linux confinement even if its environment is cleared. If
Bubblewrap cannot create a nested mount namespace, provider startup stops at
the confinement check. The browser may also fail to prepare its dedicated
account or private mount namespace for the same reason.

Use an authorized host operator session or a dedicated lab machine to start the
drill. On a systemd host, the operator can run the drill command as a dedicated,
time-bounded transient service with `systemd-run`. Give it a fresh environment,
its own `CHARIOX_HOME`, separate local ports and sockets, and fixture accounts.
Use the host's normal service account and Bubblewrap policy. Run the provider
through the drill kernel and stop the dedicated service after the drill.

If the same check fails from the host service, have the operator check the
AppArmor policy for Bubblewrap's user namespaces and the service's effective
namespace restrictions and capabilities. Keep the production provider
confinement policy in place.

### Owner agent account-copy commands

Discover these through command search; no always-loaded tool is added.
`provider accounts copy-offers` lists source profile metadata and pinned keys.
`provider accounts copy-targets` lists the owner's enrolled target kernel IDs
and pinned keys. An enrolled target may be offline; the copy checks it live.

From the receiving kernel, run `provider accounts copy-from <provider> <profile>
<source-kernel> <source-key-thumbprint> <operation-id> [--replace]`. It uses the
existing recipient consent card. From the source kernel, use `provider accounts
copy-to <provider> <profile> <target-kernel> <target-key-thumbprint> <operation-id>
[--replace]`. Keep the operation ID and pinned key when retrying; use a new
operation ID for a different copy. Both commands return metadata-only receipts
and retain the protocol's existing ownership, verification and replacement rules.
