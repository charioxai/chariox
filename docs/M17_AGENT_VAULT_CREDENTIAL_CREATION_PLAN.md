# M17 Agent Vault Credential Creation Plan

## Goal

Allow Chariox agents to create usable vault-backed credential handles on behalf of
the user without receiving secret values in model context, provider transcripts,
history, logs, or runtime MCP results.

M17 adds two runtime MCP credential-creation paths:

- `chariox.create_generated_credential`: the kernel generates a random secret,
  stores it in the configured Chariox vault, and registers matching credential
  metadata.
- `chariox.request_credential_secret`: the agent asks the user for a secret
  through a redacted Chariox runtime interaction; the kernel captures the value,
  stores it in the configured Chariox vault, and registers matching credential
  metadata.

Both tools must reuse the existing Chariox credential model. They must not define
new credential injection structures.

## Existing Credential Model

M17 must use `UserCredentialConfig` and the existing injection variants:

- `header`: inject into an HTTP header, with `name` and `value`; `value` can
  include `${secret}`.
- `query`: inject into a query parameter.
- `basic`: HTTP Basic auth, with optional `username`.
- `hmac`: sign an HTTP request with configured timestamp/signature headers.
- `pty`: write the secret directly to the current provider PTY stdin.
- `browser`: paste/fill the secret into a browser/slice field.

Credential metadata remains stored in the existing credential registry. Secret
values remain stored in the configured platform vault through
`RuntimeSecretService`.

## Non-Goals

- No Cloud-side secret storage.
- No relay-side secret inspection or proxying.
- No generic secret-returning popup tool.
- No copying home vault secrets to remote worker kernels.
- No provider-specific credential store ownership.
- No new injection policy model.

## Runtime MCP Tools

### `chariox.create_generated_credential`

The agent supplies credential metadata and optional generator settings. The
kernel generates the secret value.

Example input:

```json
{
  "credential": {
    "id": "gmail-password",
    "description": "Generated password for Gmail account",
    "allowed_hosts": ["accounts.google.com"],
    "allowed_uses": ["browser"],
    "injection": { "kind": "browser" }
  },
  "generator": {
    "kind": "password",
    "length": 32,
    "symbols": true,
    "avoid_ambiguous": true
  },
  "overwrite": false
}
```

Rules:

- If `credential.source` is omitted, infer
  `{ "type": "vault", "key": credential.id }`.
- If `credential.source` is supplied, it must be `vault`.
- Reject `env` and `file` sources.
- Generate the secret using kernel-side CSPRNG.
- Store the secret through `RuntimeSecretService::set_vault_secret`.
- Upsert metadata through the existing credential registry.
- Return only non-secret status, credential id, and vault key.

Example output:

```json
{
  "credential_id": "gmail-password",
  "vault_key": "gmail-password",
  "stored": true,
  "generated": true
}
```

### `chariox.request_credential_secret`

The agent supplies credential metadata and a user-facing prompt. The user enters
the secret through a redacted runtime interaction.

Example input:

```json
{
  "credential": {
    "id": "gmail-app-password",
    "description": "User-entered Gmail app password",
    "allowed_hosts": ["accounts.google.com"],
    "allowed_uses": ["browser"],
    "injection": { "kind": "browser" }
  },
  "prompt": {
    "title": "Add Gmail password",
    "message": "Enter the password to store in Chariox Vault.",
    "placeholder": "Password",
    "min_length": 8,
    "max_length": 256
  },
  "overwrite": false
}
```

Rules:

- Use the same `UserCredentialConfig` handling as generated credentials.
- Create one kernel-owned runtime interaction with a secret/redacted custom input.
- The typed secret must be delivered only to the kernel interaction resolver.
- The runtime MCP result must not include the secret.
- The agent receives only `stored`, `cancelled`, or `timed_out` status plus
  credential id and vault key.

Example output:

```json
{
  "credential_id": "gmail-app-password",
  "vault_key": "gmail-app-password",
  "status": "stored"
}
```

## Runtime Interaction Changes

Extend the existing runtime interaction custom choice shape with an input type:

```json
{
  "id": "secret",
  "label": "Secret",
  "placeholder": "Password",
  "min_length": 8,
  "max_length": 256,
  "input_kind": "secret"
}
```

`input_kind` defaults to `text` for backward compatibility.

Only credential-storage tool handlers may create `input_kind: "secret"`
interactions. Generic `chariox.request_popup` continues to return custom replies
to the agent and must not expose a secret input mode.

Secret interaction invariants:

- TUI renders masked input, not literal characters.
- Web renders `<input type="password">`.
- Clients clear local input state immediately after submission.
- Kernel validates min/max length without logging the value.
- Kernel stores the value and returns only non-secret status.
- Secret value is not serialized into session projection, transcript, history,
  operational logs, runtime MCP result, relay payloads beyond the direct
  encrypted kernel response path, or provider context.

Because this changes the serialized runtime interaction shape, M17 must bump the
local daemon protocol version and update protocol snapshot/hash tests.

## Policy

Credential creation must use a centralized policy hook, not hard-coded provider
permission checks:

```rust
can_agent_manage_user_vault(session, agent, runtime_policy)
```

Current default policy may allow default/full-rights agents, matching current
M16 extension-management behavior. The decision must remain centralized and
configurable so it can be tightened before launch without rewriting tool
handlers.

User-entered credential creation always requires the redacted user interaction,
even when the calling agent is allowed to manage the vault.

## Storage Semantics

Add one shared helper for atomic-as-practical credential creation:

```rust
upsert_vault_backed_credential_with_secret(credential, secret, overwrite)
```

Behavior:

- Validate credential metadata using existing credential validation.
- Require vault source.
- Respect overwrite policy for existing metadata and vault key.
- Write secret value to vault.
- Upsert credential metadata.
- If metadata upsert fails after vault write, attempt to delete the newly written
  vault value.
- Return only non-secret metadata/status.

The helper should be usable by both runtime MCP tools and future direct kernel
flows.

## Remote And Cloud Behavior

Cloud must not store secrets. The web terminal only renders the redacted
interaction and submits the user response to the connected kernel.

For remote agents:

- Home/user kernel owns home vault credential creation.
- Home secrets are not copied to worker kernels.
- Worker agents use home-owned credentials through home-authorized credential
  proxy paths. Browser and PTY injections receive only one-operation secret
  material after the worker validates the local target and home validates the
  leased-agent binding and credential policy.
- If the active worker kernel is genuinely the user's local authority for that
  session, it may create credentials in its own local vault.

This follows the same home-owned credential principle used for remote extension
execution.

## Implementation Steps

1. Add credential-creation argument/result types to runtime tools.
2. Add tool specs and canonical aliases for the two new tools.
3. Add secret input kind to runtime interaction custom choices.
4. Bump local daemon protocol version and update protocol snapshots.
5. Implement generated password generation in kernel code with CSPRNG.
6. Implement vault-backed credential upsert helper.
7. Implement `chariox.create_generated_credential`.
8. Implement `chariox.request_credential_secret`.
9. Update TUI interaction rendering/input handling for masked secret custom
   input.
10. Update web terminal interaction normalization, rendering, DOM handling, and
    tests for password input.
11. Add focused unit tests and integration tests.
12. Add live drills with screenshots and history assertions.

## Tests

OSS/kernel:

- Runtime interaction `input_kind` defaults to `text`.
- Secret input interaction validates min/max length.
- Secret interaction resolution does not expose the secret in runtime MCP result.
- Generated credential stores a retrievable vault secret with a mock vault store.
- User-entered credential stores a retrievable vault secret with a mock vault
  store.
- `chariox.list_credential_handles` shows created credential metadata without
  values.
- Existing secret-use tools can use the new handle.
- Policy hook blocks credential creation when configured false.
- Protocol snapshot/hash tests cover the interaction shape change.

CLI/TUI:

- Secret custom input renders masked.
- Typed secret is not displayed in the interaction strip.
- Local reply buffer clears after submit.
- Normal text custom popup behavior is unchanged.

Web/Cloud:

- Runtime interaction normalization preserves `input_kind`.
- Secret input renders as `<input type="password">`.
- DOM controller submits the secret value to the kernel.
- DOM/controller clears local state after submission.
- Existing Vault panel credential metadata/secret flows continue to pass.

## Live Drills

All drill artifacts should be written under `.artifacts`.

### TUI Generated Credential Drill

For each provider available at validation time:

- Codex
- Claude
- OpenCode, once account credit is available

Flow:

1. Start a local Chariox TUI session.
2. Spawn provider agent.
3. Ask agent to call `chariox.create_generated_credential` for a confined local
   browser or PTY credential.
4. Ask agent to use the handle through an existing secret-use tool.
5. Assert history contains only handle/status/tool names, not the secret.
6. Capture screenshot proving the credential was created and used.

### TUI User-Entered Credential Drill

For each provider available at validation time:

1. Start a local Chariox TUI session.
2. Ask agent to call `chariox.request_credential_secret`.
3. The validator enters the secret as the end user through the redacted field.
4. Ask agent to use the created handle.
5. Assert history contains no secret.
6. Capture screenshot proving redacted input and successful use.

### Web Terminal Generated Credential Drill

For each provider available at validation time:

1. Start kernel and web terminal.
2. Spawn provider agent through web.
3. Repeat generated credential creation and use.
4. Capture browser screenshot proving success.
5. Assert web transcript/history contains no secret value.

### Web Terminal User-Entered Credential Drill

For each provider available at validation time:

1. Start kernel and web terminal.
2. Ask agent to request a credential secret.
3. The validator enters the secret through the web password field.
4. Ask agent to use the created handle.
5. Capture browser screenshot proving redacted field and successful use.
6. Assert no secret value appears in transcript/history/tool results.

## Final External Gmail Drill

After confined drills pass:

1. Launch a slice-backed session and agent.
2. Ask the agent to open the browser and navigate to Gmail account creation.
3. When the account flow prompts for a password, ask the agent to call
   `chariox.request_credential_secret`.
4. The validator enters the password through the redacted Chariox prompt.
5. The agent uses the stored browser credential to fill the password field.
6. If Google allows account creation through the live browser flow, complete
   setup.
7. Create a new email and send it to `chariox.fortytwo@gmail.com` confirming the
   flow works.

If Google requires CAPTCHA, phone verification, identity proof, recovery-account
ownership, or another non-automatable/manual verification step, the drill stops
and records the blocker. Chariox must not bypass Google protections or fabricate
verification.

## Completion Criteria

- Runtime MCP tools create vault-backed credential handles without exposing
  secret values.
- Generated and user-entered credential paths both pass confined drills.
- Existing credential-use tools can use newly created handles in the same
  provider session.
- TUI and web terminal both support redacted secret entry.
- Cloud stores no secrets.
- Remote behavior keeps home secrets home-owned.
- Protocol version and snapshots are updated for the interaction shape change.
- Screenshots and history artifacts prove behavior for each validated provider.
