# M10 Secret Handoff Spike Plan

This spike validates whether Arroba can let agents use secrets without placing
the secret values in the model context window. It is intentionally outside
production `apps/` and `packages/` code until the behavior is proven.

## Goal

Prove these v1 claims:

- Env-source credentials can be read by the Arroba-side runtime while being
  omitted from agent/provider process environments.
- Agents can trigger controlled secret use through runtime tools without
  receiving the secret value.
- Generic bearer/API-key request injection is enough for static-token APIs.
- A generic signing adapter can sign requests without exposing the signing key.
- A terminal/console password prompt can receive a secret directly from the
  runtime without passing that value through the agent.

## Non-Goals

- No full security jail.
- No audit milestone work.
- No redaction layer.
- No MCP-specific work. Normal MCP use already keeps env secrets inside the MCP
  process unless the MCP itself returns/logs them.
- No OAuth lifecycle implementation in this spike.
- No production Arroba runtime MCP integration until the spike passes.

## Prototype Location

```text
experiments/secret-handoff/
  README.md
  package.json
  src/
    secret-harness.mjs
    fake-agent.mjs
    password-prompt.mjs
  test/
    secret-handoff.test.mjs
```

The harness simulates the target Arroba boundary:

```text
kernel/runtime process has secret sources
agent child process receives scrubbed env
agent asks runtime to use a credential handle
runtime resolves secret and performs the operation
agent receives only non-secret success/result data
```

## Spike Scenarios

### S1 Env Scrubbing

The parent runtime has env values such as `OPENAI_API_KEY`, `GITHUB_TOKEN`, and
`DB_PASSWORD`. It launches a fake agent with a sanitized environment. The fake
agent tries to inspect env values.

Success:

- Configured credential env names are absent from the child env.
- Common non-secret process env needed to run tools remains present.

### S2 Bearer/API-Key Request

The agent requests:

```json
{
  "credential_id": "openai",
  "method": "GET",
  "url": "http://127.0.0.1:<port>/echo"
}
```

The runtime injects `Authorization: Bearer <secret>` and performs the request.

Success:

- Test server receives the expected header.
- Tool result returned to the fake agent contains status/body only, not the
  credential value.
- Wrong host is rejected before secret injection.

### S3 Generic HMAC Signing

The runtime signs a request using a configured HMAC credential and sends the
signature headers to a local verification server.

Success:

- Server verifies the signature.
- Agent receives no signing key.
- Wrong host is rejected before signing.

### S4 Terminal Secret Input

A child process prints `Password:` and waits for stdin. The runtime watches the
output pattern and writes the secret bytes directly to stdin.

Success:

- Prompt process receives the password.
- Runtime result is only `{ submitted: true }`.
- The fake agent never receives the secret value.

### S5 Live Service Drill

The spike includes an optional GitHub drill:

```bash
cd experiments/secret-handoff
npm run drill:github
```

The drill sources a credential from `GITHUB_TOKEN`, `GH_TOKEN`, or
`gh auth token`, then calls `GET https://api.github.com/user` through the same
runtime injection path used by the local tests.

Success:

- GitHub returns `200`.
- The fake agent cannot read `GITHUB_TOKEN` or `GH_TOKEN`.
- The drill output contains status/login/source metadata, not the token.
- Missing local GitHub auth skips the drill without affecting unit tests.

## Progress

- M10.1 spike harness: complete.
- M10.2 deterministic local drills: complete.
- M10.3 live GitHub bearer-token drill: complete.

Validated commands:

```bash
cd experiments/secret-handoff
npm run check
npm test
npm run drill:github
```

The live GitHub drill returned `200` from `GET https://api.github.com/user`
using a token sourced from `gh auth token`; the fake agent environment could
not read `GITHUB_TOKEN` or `GH_TOKEN`, and the runtime result did not contain
the token.

## Integration Plan

### M10.4 Credential Config Model

Add Arroba config support for credential handles in the TOML config. V1
credential handles are runtime-owned references; agents see handle names and
descriptions, never values.

Proposed shape:

```toml
[[credentials]]
id = "github"
description = "GitHub API token"
source = { type = "env", name = "GH_TOKEN" }
allowed_hosts = ["api.github.com"]
allowed_uses = ["http"]

[credentials.injection]
kind = "header"
name = "authorization"
value = "Bearer ${secret}"
```

Supported v1 sources:

- `env`: read from the Arroba kernel process environment.
- `file`: read from a local file path owned by the user.

Supported v1 injections:

- `header`
- `query`
- `basic`
- `hmac`

`vault` remains a model concept from the spike but should wait until Arroba has
a real local encrypted store or OS keychain integration.

Acceptance:

- config parses and validates handles
- duplicate ids are rejected
- malformed host/use/injection settings return structured config errors
- no secret values are returned by config/status APIs

### M10.5 Provider Environment Scrubber

Scrub Arroba-launched provider processes so credential env values are available
to the kernel/runtime but not to provider child processes.

Rules:

- remove every env var referenced by configured credentials
- remove common secret-looking names by default (`*_TOKEN`, `*_SECRET`,
  `*_PASSWORD`, `*_API_KEY`, `*_PRIVATE_KEY`, `*_ACCESS_KEY`)
- preserve necessary non-secret runtime env such as `PATH`, `HOME`, `TMPDIR`,
  locale variables, and provider-specific safe env needed for launch
- apply to local providers first, then remote provider launch paths

Acceptance:

- Codex/OpenCode launch paths receive scrubbed env
- configured token env vars are absent from provider subprocess tests
- existing provider launch drills still pass

### M10.6 Runtime Secret Service

Add a kernel-owned secret service responsible for resolving credential handles,
enforcing policy, injecting credentials, and returning non-secret results.

Responsibilities:

- resolve `env` and `file` sources
- enforce `allowed_hosts` before injection/signing
- enforce `allowed_uses`
- execute HTTP requests for `http_request_with_credential`
- compute generic HMAC signatures
- never serialize secret values into history, transcript, event logs, or tool
  results

Acceptance:

- unit tests mirror the spike local HTTP/HMAC drills in Rust
- wrong-host and wrong-use requests fail before the secret is read/injected
- logs/history contain handle ids and policy errors only

### M10.7 Runtime MCP Tools

Expose the secret service through Arroba runtime MCP tools so agents can request
credential use without receiving values.

V1 tools:

- `list_credential_handles`
- `http_request_with_credential`

The terminal paste tool is useful but should be integrated after the HTTP path
because it needs tighter coordination with the existing PTY/terminal ownership
model:

- `send_secret_to_terminal`

Acceptance:

- local agent can discover handles
- local agent can call GitHub or a local HTTP drill through a handle
- tool responses omit secrets
- denied calls surface clear errors in the same place as other runtime MCP
  errors

### M10.8 Terminal Secret Handoff

Integrate password prompt handoff with the kernel terminal/PTY manager.

Scope:

- write a configured credential value to a targeted terminal/PTY stdin after a
  pattern match such as `Password:`
- return only submission status
- do not echo or store the value

Acceptance:

- local PTY drill can complete a password prompt
- no prompt transcript/history includes the password
- failure to match the prompt returns a timeout/error without printing the
  secret

### M10.9 Remote Semantics

For v1, the home kernel owns credential truth. Remote agents can request
credential use through the home runtime path, but no automatic remote secret
installation is performed.

Rules:

- HTTP credential calls execute from the kernel that owns the credential
  source unless explicitly configured later
- remote provider env is scrubbed using the same rules as local providers
- terminal handoff for remote terminals is deferred until terminal ownership
  and transport semantics are explicit

Acceptance:

- remote provider env scrub tests pass
- remote agent can request a home-owned HTTP credential handle
- docs clearly state that remote-local network locality is not solved in v1

### M10.10 Live Drills

Run end-to-end drills after each production slice:

- local GitHub API handle through a Codex agent
- local GitHub API handle through an OpenCode agent
- wrong-host denial from an agent
- secret env absence inside provider environment
- local terminal password prompt handoff
- remote provider env scrub
- remote HTTP credential request through home kernel

## Decision Gate

The spike transfer gate is passed:

- env-source secrets are unavailable to fake agent processes
- controlled HTTP/HMAC/terminal operations succeed
- host/use policy blocks wrong-target use before secret injection
- implementation stays small enough to map cleanly to Arroba runtime MCP and
  provider launch boundaries

Production integration is complete only when M10.4-M10.10 pass and docs/drills
are updated after each slice.
