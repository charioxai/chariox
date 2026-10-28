# OpenCode account status

Investigation: 2026-10-28, installed official OpenCode CLI **1.19.0**.
MP-08 / MP-09 / MP-10 / MP-11: these observations and their projection use the
same provider adapter on ordinary and managed kernels. They add no shutdown,
relay, account-transfer or credential authority.

## What the official harness exposes

`opencode --version` and `opencode auth list` were checked with an empty,
isolated HOME and XDG directories. The latter reports provider names and
credential methods, with a credential count. It does not establish that a saved
key still works. Version 1.19.0 does not support `auth list --format json`;
that command prints help rather than a machine-readable account response.
Never parse that help as successful authentication.

The [official CLI documentation](https://opencode.ai/docs/cli/) describes
`auth list`, `models`, and `stats`. The [official server API](https://opencode.ai/docs/server/)
provides `GET /provider` (catalogue, defaults and connected providers) and
`GET /provider/auth` (available authentication methods). Neither is an account
balance or subscription API. A connected provider means its configuration is
available; it is not a fresh remote credential verification.

The installed version uses the documented provider-local auth store under its
XDG data directory. The kernel's existing adapter owns reading that format;
research and clients must not open it. Only allowlisted metadata leaves the
adapter. The [newer provider documentation](https://opencode.ai/v2/docs/cli/providers/)
describes additional account/storage behaviour. A future upgrade needs a fresh
harness check; newer documentation is not proof that 1.19.0 implements it.

## Provider matrix

The catalogue is dynamic. `/provider` is the exhaustive list for the installed
server and project, including custom IDs; a fixed list would omit future and
user-defined providers. Every entry belongs to one of these cases:

| Providers available through OpenCode | Login/configuration state | Plan | Balance/credits | Rate-limit or usage windows |
| --- | --- | --- | --- | --- |
| OpenCode Go (`opencode-go`) | Saved service, credential method; no remote validation from listing | Subscription service can be identified, but not the person's active plan/tier | Not exposed by the supported status interfaces | Not exposed by the documented CLI/server status interfaces; see existing adapter below |
| OpenCode Zen/pay-as-you-go (`opencode`) | Saved service and credential method | Identifies the prepaid service, not a confirmed account plan | No documented harness balance read | No documented harness window read |
| Direct providers: OpenAI, Anthropic, Google/Gemini, xAI, DeepSeek, Mistral, Groq, Cohere and every other direct catalogue entry | Provider ID and saved/configured method; native OAuth does not promise email or account identity | Not exposed generically | Not exposed generically | Not exposed generically |
| Cloud endpoints: AWS Bedrock, Azure OpenAI, Google Vertex, Cloudflare Workers AI | Configuration/connected state; ambient credential chains need separate native verification | Not exposed | Not exposed | Not exposed |
| Gateways: OpenRouter, Vercel AI Gateway, Cloudflare AI Gateway, Eden AI and other gateway IDs | Gateway configuration and credential method | Not exposed | Not exposed | Not exposed |
| Local, self-hosted and custom OpenAI-compatible providers | Configured ID, models and connection configuration; auth may not be required | No standardized field | No standardized field | No standardized field |

The [provider setup documentation](https://opencode.ai/docs/providers/) describes
these integrations and custom endpoints. Multiple services in one OpenCode
profile are independent configurations, not proof of a single upstream account.
Two profiles with the same display name must never be merged by name. OpenCode
does not provide a universal personal identity that proves two separate stores
belong to the same account.

## Usage, cost and limitations

`opencode stats` describes locally recorded sessions, token counts and estimated
cost. It does not measure remaining credits or a subscription window. Running
status/list/catalogue commands requires no inference turn; listing costs local
process/server work. Remote credential checks or provider-specific billing
reads would have separate network cost and availability and are not implemented
by this account-menu change.

The existing kernel adapter includes a Go read at
`https://opencode.ai/zen/go/v1/usage`, mapping rolling, weekly and monthly
windows. This endpoint was not found in the official CLI/server documentation;
its existence in existing source is not a supported public contract. This work
adds no new direct requests to it. When the existing adapter has an observed
window the menu can display it; absent windows do not mean zero usage.

The [Go documentation](https://opencode.ai/docs/go/) directs people to the
Console for current usage. The [Console billing documentation](https://opencode.ai/v2/docs/console/billing/)
provides account billing controls, but does not document a general harness
balance endpoint. Console-only information must remain unavailable unless
OpenCode exposes a supported official interface. No Console scraping, SDK,
third-party service, credential copying, token decoding or secret fingerprinting
is needed for login metadata.

## Kernel and menu projection

The existing adapter records `auth_state` and safe `services` in the account
registry. `GetProviderUsageSnapshot` now carries those facts alongside usage.
The menu can say **Not signed in**, **Sign-in not checked**, **Sign in again**,
or **Login saved**, with configured services in the row’s tooltip, without inventing a balance.
An empty native default profile is not a linked account and is omitted. Explicit
profiles remain visible with their real status. Authenticated profiles with no
provider usage remain distinct from unconfigured profiles.

The kernel issues an opaque account identity from the provider-reported email
when one exists, otherwise from the machine and native profile scope. Clients
merge reports with that identity, never names or credential values. Different
stores without a provider-reported identity remain distinct; the harness cannot
prove they represent the same upstream login.

## Owner questions

Should a configured OpenCode service read **Signed in** or **Login saved** when
only credential presence is known? The latter is more precise because a saved
key may have expired. The implementation uses **Login saved** for OpenCode;
no remote verification or plan claim is implied. The accompanying status-only
mock uses the existing account row and usage text slot, with no new component.
