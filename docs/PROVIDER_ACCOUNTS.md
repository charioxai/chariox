# Provider accounts

Chariox supports multiple named account profiles for Codex, Claude, and OpenCode. The kernel owns profile metadata, selection, status/usage projection, and orchestration. Provider CLIs remain the credential-format and token-refresh authority.

## User workflow

Open **Provider Accounts** in either waiting room to list, create, link, rename, set a default, refresh, log in, log out, remove, or explicitly delete a managed profile. The launch form selects Provider, Account, Model, and Variant. New agents inherit the selected account unless another profile is chosen.

Changing an existing agent's account uses the same bounded context handoff used for provider/model changes. The active provider run ends, incompatible provider resume state is cleared, and a fresh run starts under the selected profile. Credentials are never hot-mutated in a running provider process.

The configured Cloud owner and local TUI share the home kernel's account registry. Collaborators retain separate namespaces and cannot list, use, or receive the host owner's profiles.

New managed Machines default to every authenticated, transferable profile discovered by the selected source kernel. The user may exclude profiles or disable account transfer. Before asking Cloud to create the Machine, the source kernel exports each selected profile into its provider-native portable credential shape. A missing or non-portable credential stops the launch before compute is rented. The managed-context plan still records an explicit canonical profile list; Cloud never receives credential contents.

## Provider roots

- Codex: every profile has a distinct `CODEX_HOME`. Managed profiles force `cli_auth_credentials_store = "file"`; `auth.json`, app-server processes, catalogs, usage, login, and logout are profile-scoped.
- Claude: managed and directory-linked profiles use an explicit `CLAUDE_CONFIG_DIR`. An effective native default registered without that variable preserves its absence. These are different credential scopes on macOS, even when the explicit directory is `$HOME/.claude`. Chariox invokes the official `claude auth login`, `logout`, and `status` commands in the selected scope.
- OpenCode: every profile has distinct data, config, state, cache, and `OPENCODE_CONFIG_DIR` roots. A profile may contain multiple upstream connections. Upstream usage is a capability matrix: local stats and supported native seams are projected, while unknown billing providers report unavailable rather than guessing or reading secrets for third-party APIs.

Existing effective default roots migrate once into the durable registry with stable profile IDs and public labels. `default` resolves the currently selected default; it is not a replacement for a stored profile ID. Static provider-profile configuration is not a second source of truth.

### Claude native login and legacy profiles

A successful native `claude auth status` does not prove that a directory-linked Chariox profile is signed in. First compare the selected profile's credential scope with the native invocation. Preserve the real macOS HOME. Do not copy credentials, log out, or start another login merely because the two status results differ.

Legacy registries did not record whether `$HOME/.claude` was an ambient default or an explicit `CLAUDE_CONFIG_DIR`. Migration preserves explicit scope for those ambiguous records rather than silently switching accounts. Refreshing status or choosing that profile as the default does not change its scope. Linking `$HOME/.claude` also creates an explicit directory scope, not an ambient-native account.

The current account-management commands do not provide an explicit ambient-native re-import or scope-rebinding operation for an existing registry. Treat that as a recovery limitation when an ambiguous legacy profile cannot use the native login. Do not edit a running kernel's registry file or remove/recreate profiles that existing agents depend on. Registration removal preserves provider files but does not preserve references to the removed profile ID.

## Workers and slices

The home kernel remains authoritative. When an agent is assigned to a trusted home-worker or home-managed slice, only its selected profile is materialized through the existing encrypted kernel-to-worker channel. Separate profiles use separate roots. Cloud and the relay receive only opaque encrypted packets and safe materialization status.

Materialization is denied before launch when the existing trust/ownership policy does not authorize credential transfer. A credential replica is refreshed by rematerializing from the home authority; it does not become an independent credential source. On macOS, Claude Code scopes Keychain credentials to `CLAUDE_CONFIG_DIR`. Chariox resolves that exact scoped item and converts it to Linux `.credentials.json` automatically. The legacy unscoped Keychain item is a default-profile fallback only. Empty refresh tokens are not transferable.

Model catalogs are cached by owner, selected profile, and execution location. Remote/slice selections must have a kernel-projected materialization record; clients never infer availability from labels.

## Usage semantics

Usage meters identify their source, kind, unit, limits/balance where exposed, reset time, freshness, and availability. Missing numbers mean the provider did not expose them; they are not treated as zero. Codex subscription windows and credits use app-server methods. Claude uses provider-native rate-limit observations and an explicit official-CLI `/usage` refresh with tools disabled and session persistence disabled. The refresh accepts only structured results with all required model-activity fields present and zero; missing fields are not assumed to be zero. Chariox does not rewrite Claude's onboarding or trust settings for this probe. OpenCode billing remains best-effort and extensible per upstream provider.

## Security and deletion

Profile paths and credential values are private kernel state and never appear in waiting-room, Cloud, relay, or protocol projections. Ambient provider API-key variables are scrubbed from managed launches so a named profile cannot silently execute under unrelated environment credentials.

Logout, deregistration, and deletion reject profiles with active runs. Removing a profile keeps provider data. Deleting managed data is a separate operation requiring the exact profile ID and is unavailable for linked/default roots.

## Live validation

`apps/cli/scripts/live-multi-account-drill.mjs` is opt-in and accepts existing profile IDs. It never copies or prints credentials. OpenCode remaining-balance validation is intentionally pending until suitable accounts/upstreams are available; unsupported sources remain visible as unavailable capability states.
