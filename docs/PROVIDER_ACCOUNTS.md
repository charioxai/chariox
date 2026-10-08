# Provider accounts

Chariox supports multiple named account profiles for Codex, Claude, and OpenCode. The kernel owns profile metadata, selection, status/usage projection, and orchestration. Provider CLIs remain the credential-format and token-refresh authority.

## User workflow

Open **Provider Accounts** in either waiting room to list, create, link, rename, set a default, refresh, log in, log out, remove, or explicitly delete a managed profile. The launch form selects Provider, Account, Model, and Variant. New agents inherit the selected account unless another profile is chosen.

Changing an existing agent's account uses the same bounded context handoff used for provider/model changes. The active provider run ends, incompatible provider resume state is cleared, and a fresh run starts under the selected profile. Credentials are never hot-mutated in a running provider process.

The configured Cloud owner and local TUI share the home kernel's account registry. Collaborators retain separate namespaces and cannot list, use, or receive the host owner's profiles.

New managed Machines default to every authenticated, transferable profile discovered by the selected source kernel. The user may exclude profiles or disable account transfer. Before asking Cloud to create the Machine, the source kernel exports each selected profile into its provider-native portable credential shape. Claude credentials are accepted only when they contain a non-empty refresh token; on macOS the exact profile-scoped Keychain item is used when the profile directory has no portable credential file, with the legacy unscoped item allowed only for the default profile. A missing or non-portable credential stops the launch before compute is rented. The managed-context plan still records an explicit canonical profile list; Cloud never receives credential contents.

## Provider roots

- Codex: every profile has a distinct `CODEX_HOME`. Managed profiles force `cli_auth_credentials_store = "file"`; `auth.json`, app-server processes, catalogs, usage, login, and logout are profile-scoped.
- Claude: managed and directory-linked profiles use an explicit `CLAUDE_CONFIG_DIR`. An effective native default registered without that variable preserves its absence. These are different credential scopes on macOS, even when the explicit directory is `$HOME/.claude`. Chariox invokes the official `claude auth login`, `logout`, and `status` commands in the selected scope.
- OpenCode: every profile has distinct data, config, state, cache, and `OPENCODE_CONFIG_DIR` roots. A profile may contain multiple upstream connections. Upstream usage is a capability matrix: local stats and supported native seams are projected, while unknown billing providers report unavailable rather than guessing or reading secrets for third-party APIs.

Existing effective default roots migrate once into the durable registry with stable profile IDs and public labels. `default` resolves the currently selected default; it is not a replacement for a stored profile ID. Static provider-profile configuration is not a second source of truth.

Linking a provider directory already registered under the same owner and provider returns an error identifying the existing profile. Duplicate authenticated identities across separate directories remain rejected by refresh.

Profiles created without a label follow the authenticated account email's local part. A rename makes the label user-owned, even when the chosen label equals the current automatic name. Colliding automatic labels receive a numeric suffix; reserved or invalid email names leave the last label unchanged. Legacy profiles without naming metadata treat provider-number aliases and names matching the last observed email local part as automatic; other labels are preserved.

### Claude native login and legacy profiles

A successful native `claude auth status` does not prove that a directory-linked Chariox profile is signed in. First compare the selected profile's credential scope with the native invocation. Preserve the real macOS HOME. Do not copy credentials, log out, or start another login merely because the two status results differ.

Legacy registries did not record whether `$HOME/.claude` was an ambient default or an explicit `CLAUDE_CONFIG_DIR`. Migration preserves explicit scope for those ambiguous records rather than silently switching accounts. Refreshing status or choosing that profile as the default does not change its scope. Linking `$HOME/.claude` also creates an explicit directory scope, not an ambient-native account.

Use `/provider accounts import-native <provider>` to register the kernel host's current native scope explicitly. Import is idempotent for a scope already registered. When the native scope differs from an ambiguous legacy profile, it creates a separate stable profile and does not replace the old registration, change the default, start login, or copy provider files. Select or mark the imported profile as default only after its native status is refreshed. Do not edit a running kernel's registry file or remove/recreate profiles that existing agents depend on. Registration removal preserves provider files but does not preserve references to the removed profile ID.

## Workers and slices

The home kernel owns the agent and selects the account profile used for its initial placement. When an agent is assigned to a trusted home-worker or home-managed slice, only that selected profile is installed through the existing encrypted kernel-to-worker channel. Separate profiles use separate roots. Claude uses its provider-native portable `.credentials.json` shape after refreshability validation. Cloud and the relay receive only opaque encrypted packets and safe installation status.

Installation is denied before launch when the existing trust/ownership policy does not authorize credential transfer. After the target accepts the profile, that kernel owns its files and provider-native state exactly like a locally created profile. Later agent launches reuse it without exporting or retransmitting source credentials. Source-side login, logout, or account changes do not mutate the installed target profile. Replacing credentials is an explicit operation on the target kernel.

Each vaulted provider credential has a deterministic, non-secret handle derived from the account owner, provider family, and stable profile ID (`claude`, `claude-p` and `claude-headless` agents of one account share it). Launch preparation resolves that handle through `RuntimeSecretService`. Local launch keeps the resulting value in a redacted, non-serializable, zeroizing environment. Remote launch wraps it in a profile-bound redacted value inside the existing end-to-end-encrypted kernel packet; the worker immediately moves it back into the same non-serializable launch environment. It is excluded from provider-run persistence, projections, command history, and debug output. Claude's live runtime retains that protected environment only so a provider child process can restart without returning to macOS credential storage.

For MP-08 / MP-10 / MP-11, `provider setup-token claude <account-profile>` starts the official Claude authorization and token-capture flow. The advanced manual path stores a token created with the official `claude setup-token` command using `provider setup-token claude <account-profile> --paste`; input is hidden. Replacement requires `--replace`. The kernel verifies that the profile belongs to the caller's account authority, requests a Chariox Vault unlock through `RuntimeInteraction` when needed, and writes the vault value and provider-only credential policy as one operation. The setup token is never stored in the Claude profile directory or macOS Keychain.

Every setup token, pasted or captured with `--run`, is verified before storage with one minimal official-CLI turn (`claude -p` with tools disabled, no session persistence, `--model haiku`). `claude auth status` and the local `/usage` command report success for any environment token, so only an API call proves it. A token Claude rejects (HTTP 401/403: invalid, expired or revoked) stores nothing and returns an error telling the user to create a new one with `claude setup-token`; an inconclusive check also stores nothing. For MP-08 / MP-10 / MP-11, a verified token marks its credential registration with `metadata.created_by_kind: verified_provider_account_profile` and marks the profile authenticated. A legacy registration without that provenance is unchecked even if an older native login authenticated the profile; status, refresh and launch admission discard that native identity, plan and usage. Its first official-CLI verification persists the provenance across restarts. Every token replacement resets it unless the replacement itself was verified. `claude auth status` cannot see the vaulted token and reading it back needs the vault, so later status checks and refreshes keep the observation recorded at storage instead of downgrading the profile to not logged in; the registry keeps it across kernel restarts.

For MP-08 / MP-11 (post-merge), `provider setup-token claude <account-profile> --run` runs the official `claude setup-token` inside a kernel-owned PTY on the kernel that owns the profile. The TUI also accepts `/provider setup-token claude <account-profile> --run`; Web Provider Accounts offers **Create setup token** and explicit **Replace setup token** actions on protocol 411 or newer. Existing entries require `--replace` before minting. The request uses the existing `StartProviderLogin` method `setup_token`: a 1,000-column terminal render captures the complete token, the verification turn runs before storage, and the login workflow asks for the Vault passphrase only when storing it.

For MP-08 / MP-11, the existing terminal renderer projects readable provider output, including the real `https://claude.com/cai/oauth/authorize` link, while hiding credential-shaped values and authorization-code echoes. The shared screen receives raw bytes before PTY queues or diagnostic tails; only its redacted text reaches login status. Storage requires one complete `sk-ant-oat01-` candidate with at least 80 body characters, a successful CLI exit, and the verification turn above. Missing, short or multiple candidates, verification failure, timeout, cancellation and CLI failure store nothing. Status, hidden authorization-code input, Vault passphrase entry and cancel use the normal provider login workflow (`provider login-status`, `provider login-input`, `provider login-cancel`); clients and Cloud never receive the resulting token. Vault I/O runs outside the login-store mutex and unlocks only for the storage operation. The manual hidden-input command remains supported.

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

## Usage semantics

Usage meters identify their source, kind, unit, limits/balance where exposed, reset time, freshness, and availability. Missing numbers mean the provider did not expose them; they are not treated as zero. Codex subscription windows and credits use app-server methods. Claude uses provider-native rate-limit observations and an explicit official-CLI `/usage` refresh with tools disabled and session persistence disabled. The refresh accepts only structured results with all required model-activity fields present and zero; missing fields are not assumed to be zero. Chariox does not rewrite Claude's onboarding or trust settings for this probe. OpenCode billing remains best-effort and extensible per upstream provider.

## Security and deletion

Profile paths and credential values are private kernel state and never appear in waiting-room, Cloud, relay, or protocol projections. Ambient provider API-key variables are scrubbed from managed launches so a named profile cannot silently execute under unrelated environment credentials.

Logout, deregistration, and deletion reject profiles with active runs. Removing a profile keeps provider data. Deleting managed data is a separate operation requiring the exact profile ID and is unavailable for linked/default roots.

## Live validation

`apps/cli/scripts/live-multi-account-drill.mjs` is opt-in and accepts existing profile IDs. It never copies or prints credentials. OpenCode remaining-balance validation is intentionally pending until suitable accounts/upstreams are available; unsupported sources remain visible as unavailable capability states.
