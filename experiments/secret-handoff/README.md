# Secret Handoff Spike

This disposable spike proves the v1 secret handoff boundary before production
Arroba integration.

The target behavior is practical obfuscation and accidental-exposure
prevention, not a complete security jail:

```text
runtime has access to secret sources
agent process receives scrubbed env
agent requests use of credential handles
runtime injects/signs/pastes secrets outside model context
agent receives only non-secret status/result data
```

Out of scope for this spike:

- audit trail design
- redaction layer
- MCP secret proxying
- OAuth lifecycle
- production runtime MCP integration

## Commands

```bash
npm run check
npm test
npm run drill:github
```

Or from the repository root:

```bash
cd experiments/secret-handoff
npm run check
npm test
npm run drill:github
```

`drill:github` is a live-service drill. It uses `GITHUB_TOKEN`, `GH_TOKEN`, or
`gh auth token` if available, calls `GET https://api.github.com/user`, and
prints only non-secret metadata. If no GitHub credential is available, it skips
instead of failing the deterministic test suite.

## What It Validates

- env-source credentials are removed from the fake agent environment
- bearer/API-key header injection works against a local server
- generic HMAC signing works against a local verification server
- terminal password prompts can receive secret bytes directly from the runtime
- wrong-host policy blocks use before injection/signing
- optional live GitHub auth works without exposing the token to the fake agent
  process or returning it in the runtime result

## Transfer

If the tests pass, port the model into Arroba as:

- credential handles and sources
- provider launch env scrubber
- `http_request_with_credential`
- `send_secret_to_terminal`
- built-in header/query/basic/HMAC adapters
