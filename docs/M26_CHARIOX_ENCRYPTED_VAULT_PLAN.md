# M26 Chariox Encrypted Vault Plan

## Goal

Replace OS-dependent credential storage with a Chariox-owned encrypted vault that behaves consistently across macOS, Linux, and Windows.

The kernel must control all vault unlock UX through Chariox runtime interactions projected to terminals. Secrets must remain encrypted at rest, decrypted only inside the kernel, and never returned to agents.

## Product Requirements

- No macOS Keychain, Windows Credential Manager, Linux Secret Service, or per-secret OS confirmation prompts for Chariox vault operations.
- No legacy compatibility path for OS-backed secret items. Existing OS-backed secrets are not read or migrated by this milestone.
- Runtime MCP secret tools keep the same no-leak contract: agents get credential handles and tool outcomes, not secret values.
- Unlocking is a Chariox popup interaction. The popup appears in all attached terminals that can answer runtime interactions.
- Metaagents and regular agents may trigger an unlock request when a vault operation requires it, but the passphrase is captured by the kernel-owned popup and is not delivered to the agent.
- The user can choose an unlock duration from the popup, such as this operation, 30 minutes, 1 hour, this session, or until kernel shutdown.
- The user can extend an active unlock through a popup without retyping the passphrase when the vault is already unlocked.
- The user can lock the vault explicitly from TUI, web terminal, or a runtime command.

## Unlock Model

The vault is always encrypted at rest. Unlock policy controls how long the kernel may keep the decrypted vault key in memory.

Config shape:

```toml
[credentials.vault]
backend = "chariox_encrypted"
path = "~/.chariox/vault/vault.db"
unlock_policy = "ttl"
default_ttl_minutes = 30
max_ttl_minutes = 240
```

Supported policies:

- `kernel_init`: request unlock during kernel startup and keep it until shutdown.
- `session`: unlock per session and keep access for that session until it ends or expires.
- `agent`: unlock per agent and keep access for that agent until it ends or expires.
- `ttl`: unlock for a configured time window, defaulting to 30 minutes.
- `always`: require an unlock interaction for every secret operation.

The first implementation should support `ttl`, `kernel_init`, and `always`. `session` and `agent` can share the same internal scope model and be added once the base path is stable.

## Vault Storage

Add a new durable `CredentialVaultStore` backend:

- `CharioxEncryptedCredentialVaultStore`
- Encrypted database or file under the Chariox config/state directory.
- Versioned vault header with algorithm, KDF params, salt, and schema version.
- Derive the vault key from a user passphrase with a memory-hard KDF such as Argon2id.
- Encrypt secret values with an authenticated encryption scheme such as XChaCha20-Poly1305 or AES-256-GCM.
- Store metadata needed for lookup, creation time, update time, and deletion markers without exposing secret values.

The implementation should use a maintained Rust crypto crate and avoid custom cryptography beyond composition and file format.

## Runtime Interaction UX

Use existing kernel-owned `RuntimeInteraction` infrastructure.

Locked vault popup:

- Level: critical.
- Title: `Unlock Chariox Vault`.
- Message explains which session/agent/tool requested access.
- Redacted custom input for the vault passphrase.
- Choices for unlock duration: `This operation`, `30 minutes`, `1 hour`, `Until kernel shutdown`, `Cancel`.
- The response is captured only by the kernel unlock service.

Unlocked vault popup:

- Title: `Chariox Vault Unlocked`.
- Choices: `Extend 30 minutes`, `Extend 1 hour`, `Lock now`, `Dismiss`.
- No passphrase field needed while the vault key is already in memory.

All terminals attached to the session should see the same interaction. A successful response resolves the pending vault operation and wakes the blocked runtime MCP call.

## Kernel Architecture

Add a small vault coordinator owned by the kernel:

- Tracks locked/unlocked state.
- Tracks unlocked scopes and expiry timestamps.
- Deduplicates concurrent unlock requests into one interaction.
- Holds decrypted vault key material in memory only while unlocked.
- Clears key material on expiry, explicit lock, kernel shutdown, or failed integrity checks.
- Emits terminal/runtime events when vault state changes.

Secret service flow:

1. Runtime MCP tool requests a credential operation.
2. `RuntimeSecretService` asks vault coordinator for an unlocked vault handle.
3. If unlocked, operation proceeds.
4. If locked, coordinator creates or joins a pending unlock interaction.
5. The tool waits for unlock, times out, or returns a clear cancellation error.
6. Secret values are injected only through existing browser, terminal, connector, MCP, or HTTP injection paths.

Keep the existing warm in-memory secret cache, but make it subordinate to unlock state:

- Populate cache after successful encrypted vault write/read.
- Clear cache when vault locks.
- Never serialize the warm cache.

## Config And Removal Work

- Change the default vault backend to `chariox_encrypted`.
- Remove OS keychain backend selection from default config.
- Delete or quarantine platform keychain code paths used by Chariox vault operations.
- Remove tests and docs that rely on macOS Keychain prompts for normal operation.
- Keep no automatic migration from old OS-backed secrets. Users recreate secrets through the new runtime MCP/UI path.

## Security Invariants

- Agents never receive vault passphrases or decrypted secret values.
- Runtime interaction custom secret replies are never written to transcript history.
- Vault unlock events may reveal that a vault was unlocked, but not which secret value was used.
- Encrypted vault files are integrity checked before use.
- Failed unlock attempts use generic errors and should be rate-limited in the coordinator.
- Locking clears decrypted key material and warm secret cache.

## Implementation Phases

### Phase 1: Encrypted Store

- Add encrypted vault file format and store implementation.
- Add create/open/read/write/delete tests with a temporary vault file.
- Add corruption and wrong-passphrase tests.
- Wire config parsing for `chariox_encrypted`.

### Phase 2: Unlock Coordinator

- Add kernel vault state, scopes, TTL expiry, and lock/extend operations.
- Add unit tests for `ttl`, `kernel_init`, `always`, cancellation, expiry, and concurrent request dedupe.
- Ensure lock clears decrypted key material and warm cache.

### Phase 3: Popup Integration

- Implement `Unlock Chariox Vault` interaction with redacted input and duration choices.
- Implement `Chariox Vault Unlocked` extension popup.
- Project interactions to web terminal, local TUI, remote TUI, and native TUI through existing interaction plumbing.
- Add cancellation and timeout behavior with clear user-facing messages.

### Phase 4: Runtime MCP Integration

- Route `create_generated_credential`, `request_credential_secret`, `paste_secret_to_slice`, MCP secret resolution, connector secret resolution, and HTTP credential calls through the coordinator.
- Preserve the no-leak transcript contract.
- Verify same-turn warm use after credential creation.

### Phase 5: Product Drills

Run live end-to-end drills and save screenshots in `.artifacts`.

- Web terminal View tab: generated credential is created, stored, pasted into slice, and verified without OS prompts.
- Web terminal View tab: user-entered secret popup stores a credential and then pastes it into slice without leaking.
- Local TUI: locked vault triggers popup, user unlocks for 30 minutes, agent uses credential.
- Remote TUI: same unlock interaction is projected and resolved remotely.
- TTL drill: unlock for 30 minutes, use secret, extend by 1 hour, verify operation continues without passphrase, then lock and verify a new operation prompts again.
- `always` policy drill: two secret operations produce two unlock prompts.

Pass criteria:

- No OS credential dialog appears during any normal vault operation.
- Screenshots show Chariox popups, redacted input, duration choices, and successful secret use.
- Session history and agent traces do not contain passphrases or secret values.
- Agents can trigger the need for unlock but cannot read or infer the secret.

## Open Decisions

- Exact default unlock policy for launch: recommended early default is `ttl` with 30 minutes.
- Maximum TTL allowed by product policy.
- Whether `kernel_init` should block startup or allow lazy first-use unlock.
- Whether to expose lock/extend as slash commands only, UI buttons only, or both.
