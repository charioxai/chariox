# auth.md drills — MP-08 / MP-09 / MP-10 / MP-11 AU-15

`service.mjs` is an in-memory local auth.md service implementing RFC 9728 / RFC
8414 discovery, Anonymous assertion exchange, Service Auth, anonymous claim,
claim polling and Bearer API use. It never logs credentials. It is regression
infrastructure, not a substitute for public-service acceptance.

Build the source-matching kernel and hosted relay through the lane cargo slot. Build
the OSS kernel-client. In the matching Cloud checkout, build the actual new UI:

```sh
pnpm --filter @chariox-cloud/web... run build
```

Serve `apps/web/dist` behind an owned hosted validation stack with its own
relay and synthetic fixture owner. The self-hosted bootstrap currently omits the
kernel protocol; do not use it for sudo drills or repair it in this lane.

Use the existing Chariox-linked native Codex profile as `CODEX_HOME`; no account
files are copied or changed by the drill. Run inside the lane's bounded workload
wrapper, with state and evidence outside either checkout:

```sh
node scripts/e2e-stack/agent-auth/live.mjs \
  --kernel-bin /absolute/path/chariox-kernel \
  --cloud-origin https://owned-hosted-stack.example.test \
  --kernel-source-head <kernel-source-commit> \
  --web-source-head <web-source-commit> \
  --playwright-module /absolute/cloud-checkout/node_modules/playwright-core/index.mjs \
  --home /absolute/path/to/new-disposable-state \
  --evidence /absolute/path/to/evidence \
  --dpr 1
```

The fixture verifies the already linked native Codex profile through the owner
Refresh API and starts first-time Vault management through the owner API. All
Vault passphrase answers use the actual new-UI secret interaction. Those setup
initiators are absent from the current new-UI windows and are reported as gaps.

The client is exclusively `apps/web`'s new terminal-os shell, built from the real
hosted application entry (`/waiting-room?ui=os`). The drill enrolls its kernel
through the product device flow and real hosted WSS relay, submits two owner-authorized `/sudo` prompts, answers the actual new UI
passphrase/passkey controls, and uses official Codex (`gpt-6-luna`, low). The
service claim completes in a real browser page. Both flows must produce HTTP 200
using the Vault-held handle. It checks kernel/provider history, logs and page text
for the service's access tokens, identity assertions and claim tokens. A base
kernel without `chariox.agent_auth.register` fails before an authenticated API
call. The report records the kernel hash, source head, HTTP counts and absence
checks. It shuts down its own browser, kernel, relay and service and removes its
owned state. The hosted stack is owned and cleaned up separately. Repeat with `--dpr 2` when checking the client at both densities.

Public prerequisite probe (read-only; exit 2 means no supported issuer-free flow):

```sh
node scripts/e2e-stack/agent-auth/public-discovery.mjs
```

On 2026-11-02 Firecrawl's public metadata advertises only `identity_assertion`;
its auth.md explicitly disables Anonymous, Service Auth and claims. AU-15's
Firecrawl credential/API acceptance is blocked by the parked trusted issuer
work. The probe does not mint an assertion or attempt registration. Do not call
a local fixture pass a public AU-15 pass.
