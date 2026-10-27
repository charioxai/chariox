# Provider account usage sources

Status: researched 2026-08-24. This document records which provider-native
surfaces Chariox can use without scraping interactive terminal or browser UI.
Credential values must remain inside provider-owned stores and are never part
of the usage snapshot.

## Product contract

Chariox represents usage as independent meters. A meter states its source,
scope, unit, observed time, optional reset, and whether the provider actually
reported a number. Missing data is not zero and is not evidence that an account
is exhausted.

Refresh is account-specific. Every adapter runs with the selected stable
provider profile and must not follow a provider's mutable default account.
Subscription windows, prepaid balances, monthly spend, per-minute API quotas,
and local token/cost history are different meter kinds and must not be merged.

## Supported account classes

| Harness | Account class | Reliable provider-native source | Product result |
|---|---|---|---|
| Codex | ChatGPT subscription | Codex app-server `account/rateLimits/read` and `account/rateLimits/updated` | All returned rolling windows, including 5-hour and weekly windows, with reset times |
| Codex | ChatGPT credits/extra usage | Codex app-server rate-limit payload credits fields | Credit balance or spend-control state when the account exposes them |
| Codex | OpenAI API key | OpenAI response rate-limit headers and organization usage/cost APIs where the enrolled credential has the required administrative scope | API quotas and spend only; never label them as ChatGPT subscription windows |
| Claude | Claude.ai Pro/Max subscription | Claude Code status-line `rate_limits.five_hour` and `rate_limits.seven_day`, merged with native `rate_limit_event` observations | 5-hour and weekly usage/reset meters after the first provider response |
| Claude | Claude.ai subscription extra usage | Claude subscription usage response when exposed by the provider adapter | Extra-usage spend/limit meter, separate from subscription windows |
| Claude | Claude API individual key | Per-response Anthropic rate-limit headers plus local metering | Current API headroom and observed usage; no organization billing total |
| Claude | Claude Console organization | Usage & Cost Admin API, Rate Limits API, and response headers | Organization/workspace usage, cost, spend cap, and API quotas |
| Claude | Claude Enterprise | Enterprise Analytics API and Spend Limits API with an explicitly enrolled analytics/admin credential | Organization/user cost, usage credits, and effective spend limits |
| Claude | Bedrock/Vertex/AWS marketplace | The cloud provider's billing/quota API, not Anthropic credit APIs | Provider-native cloud billing only when that cloud account is enrolled |
| OpenCode | OpenCode Go subscription | `GET /zen/go/v1/usage` with the selected Zen/Go API key | Rolling, weekly, and monthly percentages and reset times |
| OpenCode | Zen prepaid credits | No key-authenticated balance endpoint exists today | Honest unsupported-capability result plus Zen billing link; local cost remains separate |
| OpenCode | Free Zen models | Model catalog marks the model free | `not_applicable` for paid quota, not a fabricated unlimited balance |
| OpenCode | Fireworks upstream | Account/quota APIs and rated billing summary when the account slug is enrolled | Account status, quotas, and spend; Fireworks does not expose remaining prepaid balance |
| OpenCode | Other upstream provider | A named, versioned upstream adapter | Provider-native meters when an adapter exists; otherwise local stats plus an explicit unsupported reason |

## Codex

The [official Codex app-server protocol](https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md)
supports ChatGPT, API-key, and device-code authentication. Its account methods
are the authority for a Codex CLI profile. Chariox already starts one app-server
inside each profile's `CODEX_HOME`; refresh must therefore call that exact
profile rather than inspect the desktop app's account.

The current normalizer needs semantic deduplication. Recent app-server payloads
can expose a convenience `rateLimits` object and the same window again under
`rateLimitsByLimitId`. Meter identity should be the limit family plus window
duration, not the JSON traversal path. User-facing labels should be `5-hour`,
`weekly`, or a provider-reported scoped label, never `primary`/`secondary`.

For API-key accounts, [OpenAI documents model API rate limits](https://developers.openai.com/api/docs/models/gpt-5.3-codex)
as request/token limits tied to the API usage tier. These are not the ChatGPT
subscription windows returned to a ChatGPT-authenticated Codex profile.

## Claude

The supported subscription seam is Claude Code's
[status-line JSON contract](https://code.claude.com/docs/en/statusline). It
defines `rate_limits.five_hour.used_percentage`,
`rate_limits.five_hour.resets_at`,
`rate_limits.seven_day.used_percentage`, and
`rate_limits.seven_day.resets_at`. The object is available only to Claude.ai
Pro/Max subscribers after the first API response, and either window may be
absent independently.

Chariox can install a per-run status-line command through the temporary Claude
settings file it already owns. The command should atomically write the latest
JSON snapshot to a private runtime file. The kernel merges that snapshot with
streamed `rate_limit_event` observations by meter identity, retaining both the
5-hour and weekly windows instead of replacing the whole account snapshot with
the newest event.

Provider Accounts refresh never reads Claude Code credential files or macOS
Keychain entries and does not call undocumented account endpoints. It refreshes
authentication through the provider CLI, retains the latest real status-line or
`rate_limit_event` observation, and projects that observation to `stale` after
`PROVIDER_USAGE_STALE_AFTER_MS`. Missing data is never presented as zero. A
later provider run restores `available` from a fresh provider-native
observation.

API and organization accounts are separate products. Anthropic's
[Usage & Cost API](https://platform.claude.com/docs/en/manage-claude/usage-cost-api)
requires an Admin API key and is unavailable to individual accounts. The
[Claude Enterprise Analytics API](https://platform.claude.com/docs/en/manage-claude/analytics-api)
uses a different Analytics API key; those key types are not interchangeable.
The [API rate-limit documentation](https://platform.claude.com/docs/en/api/rate-limits)
defines spend limits, request/token limits, and response headers, while the
[Enterprise Spend Limits API](https://platform.claude.com/docs/en/manage-claude/spend-limits-api)
reports effective member limits and period-to-date spend.

## OpenCode, Zen, and Go

[OpenCode Go](https://opencode.ai/docs/go/) is a subscription with 5-hour,
weekly, and monthly dollar-valued limits. The official open-source
[`/zen/go/v1/usage` route](https://github.com/anomalyco/opencode/blob/dev/packages/console/app/src/routes/zen/go/v1/usage.ts)
authenticates a Zen API key and returns `usage.rolling`, `usage.weekly`, and
`usage.monthly`, each with `status`, `percent`, and `resetsAt`. A 403
`EntitlementError` means the selected key has no Go subscription and is not a
zero-usage result.

[OpenCode Zen](https://opencode.ai/docs/zen/) is prepaid pay-as-you-go credit.
The key-authenticated API does not currently expose the remaining wallet
balance. The console reads the balance through an authenticated web server
function, and the public feature request
[#44189](https://github.com/anomalyco/opencode/issues/44189) documents this
missing API. Chariox must not copy browser cookies, scrape the console, or infer
the balance from local spend because usage can occur outside Chariox and
auto-reload can change it. When OpenCode exposes a key-authenticated balance,
it can be added to the same versioned adapter.

OpenCode local `stats --format json` remains useful for local tokens and cost,
but it does not prove an upstream subscription limit or remaining balance.

## Fireworks and arbitrary OpenCode upstreams

Fireworks exposes [account discovery](https://docs.fireworks.ai/api-reference/list-accounts)
and [account quotas](https://docs.fireworks.ai/api-reference/list-quotas) using
the API key. It also supports exporting comprehensive billing metrics through
[`firectl billing export-metrics`](https://docs.fireworks.ai/accounts/exporting-billing-metrics).
Its account status can distinguish credit depletion or monthly-spend-limit
exhaustion, but the documented APIs do not expose a remaining prepaid-credit
balance. Chariox can therefore show account status, quota usage, and rated
spend, but must link to Fireworks billing for the authoritative balance.

OpenCode supports many upstreams, so Chariox should use a registry of
provider-qualified usage adapters rather than a generic recursive JSON parser.
An adapter declares credential class, supported meters, management URL,
freshness, and failure semantics. Unknown upstreams continue to show local
OpenCode statistics and a precise `provider does not expose a supported billing
API` reason.

## Validation matrix

1. Contract fixtures for every response variant and negative condition.
2. Two Codex subscription accounts: refresh each stable profile and prove
   independent 5-hour/weekly meters and reset times.
3. One Claude Pro/Max account: make one minimal real turn, capture status-line
   JSON, refresh without another turn, and prove both windows persist.
4. Claude missing-window, expired-auth, and rate-limit-event merge fixtures.
5. Current Zen key: `/zen/go/v1/usage` must report the real no-subscription
   entitlement; local stats remain separate. Credit balance stays explicitly
   unsupported until OpenCode exposes an authenticated API.
6. OpenCode Go contract fixtures now; a live positive drill when the user adds
   a Go subscription.
7. Fireworks fixtures for account, quota, spend, auth failure, and missing
   account slug; live drill only when a Fireworks profile is enrolled.
8. Web and TUI evidence must show the same meters and unavailable reasons.
