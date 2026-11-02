# auth.md drills — MP-08 / MP-09 / MP-10 / MP-11 AU-15

`service.mjs` is an in-memory local auth.md service implementing RFC 9728 / RFC
8414 discovery, Anonymous assertion exchange, Service Auth, anonymous claim,
claim polling and Bearer API use. It never logs credentials. It is regression
infrastructure, not a substitute for public-service acceptance.

Build the source-matching kernel and relay through the lane cargo slot. Build
the OSS kernel-client. In the matching Cloud checkout, build the actual new UI:

```sh
CHARIOX_WEB_CONTROL_PLANE=selfhosted pnpm --filter @chariox-cloud/web... run build
```

Use the existing Chariox-linked native Codex profile as `CODEX_HOME`; no account
files are copied or changed by the drill. Run inside the lane's bounded workload
wrapper, with state and evidence outside either checkout:

```sh
node scripts/e2e-stack/agent-auth/live.mjs \
  --kernel-bin /absolute/path/chariox-kernel \
  --relay-bin /absolute/path/chariox-relay \
  --web-root /absolute/cloud-checkout/apps/web/dist/selfhosted \
  --playwright-module /absolute/cloud-checkout/node_modules/playwright-core/index.mjs \
  --home /absolute/path/to/new-disposable-state \
  --evidence /absolute/path/to/evidence \
  --dpr 1
```

The client is exclusively `apps/web`'s new terminal-os shell, built from the real
self-hosted application entry. The drill pairs it through the real encrypted
relay, submits two owner-authorized `/sudo` prompts, answers the actual new UI
passphrase/passkey controls, and uses official Codex (`gpt-6-luna`, low). The
service claim completes in a real browser page. Both flows must produce HTTP 200
using the Vault-held handle. It checks kernel/provider history, logs and page text
for the service's access tokens, identity assertions and claim tokens. A base
kernel without `chariox.agent_auth.register` fails before an authenticated API
call. The report records the kernel hash, source head, HTTP counts and absence
checks. It shuts down its own browser, kernel, relay and service and removes its
owned state. Repeat with `--dpr 2` when checking the client at both densities.

Public prerequisite probe (read-only; exit 2 means no supported issuer-free flow):

```sh
node scripts/e2e-stack/agent-auth/public-discovery.mjs
```

On 2026-11-02 Firecrawl's public metadata advertises only `identity_assertion`;
its auth.md explicitly disables Anonymous, Service Auth and claims. AU-15's
Firecrawl credential/API acceptance is blocked by the parked trusted issuer
work. The probe does not mint an assertion or attempt registration. Do not call
a local fixture pass a public AU-15 pass.
