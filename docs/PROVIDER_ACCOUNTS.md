# Provider accounts

Chariox supports multiple named account profiles for Codex, Claude, and OpenCode. The kernel owns profile metadata, selection, status/usage projection, and orchestration. Provider CLIs remain the credential-format and token-refresh authority.

## User workflow

Open **Provider Accounts** in either waiting room to list, create, link, rename, set a default, refresh, log in, log out, remove, or explicitly delete a managed profile. The launch form selects Provider, Account, Model, and Variant. New agents inherit the selected account unless another profile is chosen.

Changing an existing agent's account uses the same bounded context handoff used for provider/model changes. The active provider run ends, incompatible provider resume state is cleared, and a fresh run starts under the selected profile. Credentials are never hot-mutated in a running provider process.

The configured Cloud owner and local TUI share the home kernel's account registry. Collaborators retain separate namespaces and cannot list, use, or receive the host owner's profiles.

## Provider roots

- Codex: every profile has a distinct `CODEX_HOME`. Managed profiles force `cli_auth_credentials_store = "file"`; `auth.json`, app-server processes, catalogs, usage, login, and logout are profile-scoped.
- Claude: every profile has a distinct `CLAUDE_CONFIG_DIR`. Chariox invokes the official `claude auth login`, `logout`, and `status` commands in that environment. Usage is updated from provider-native rate-limit observations when available.
- OpenCode: every profile has distinct data, config, state, cache, and `OPENCODE_CONFIG_DIR` roots. A profile may contain multiple upstream connections. Upstream usage is a capability matrix: local stats and supported native seams are projected, while unknown billing providers report unavailable rather than guessing or reading secrets for third-party APIs.

Existing effective default roots migrate once into the durable registry as `default`. Static provider-profile configuration is not a second source of truth.

## Workers and slices

The home kernel remains authoritative. When an agent is assigned to a trusted home-worker or home-managed slice, only its selected profile is materialized through the existing encrypted kernel-to-worker channel. Separate profiles use separate roots. Cloud and the relay receive only opaque encrypted packets and safe materialization status.

Materialization is denied before launch when the existing trust/ownership policy does not authorize credential transfer. A credential replica is refreshed by rematerializing from the home authority; it does not become an independent credential source. Claude's existing macOS Keychain-to-Linux `.credentials.json` conversion remains part of this path.

Model catalogs are cached by owner, selected profile, and execution location. Remote/slice selections must have a kernel-projected materialization record; clients never infer availability from labels.

## Usage semantics

Usage meters identify their source, kind, unit, limits/balance where exposed, reset time, freshness, and availability. Missing numbers mean the provider did not expose them; they are not treated as zero. Codex subscription windows and credits use app-server methods. Claude uses structured auth plus observed rate-limit events. OpenCode billing remains best-effort and extensible per upstream provider.

## Security and deletion

Profile paths and credential values are private kernel state and never appear in waiting-room, Cloud, relay, or protocol projections. Ambient provider API-key variables are scrubbed from managed launches so a named profile cannot silently execute under unrelated environment credentials.

Logout, deregistration, and deletion reject profiles with active runs. Removing a profile keeps provider data. Deleting managed data is a separate operation requiring the exact profile ID and is unavailable for linked/default roots.

## Live validation

`apps/cli/scripts/live-multi-account-drill.mjs` is opt-in and accepts existing profile IDs. It never copies or prints credentials. OpenCode remaining-balance validation is intentionally pending until suitable accounts/upstreams are available; unsupported sources remain visible as unavailable capability states.
