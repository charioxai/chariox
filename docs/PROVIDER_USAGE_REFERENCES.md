# Provider usage references

MP-08 / MP-10 / MP-11: these references are kernel facts shared by ordinary and
managed clients. They are read-only; account management remains in Settings.

Local protocol 589 / relay peer 195 adds two reads:

- `GetProviderUsageSnapshot {}` returns `ProviderUsageSnapshot { accounts }`.
  Each account has only `provider`, `profile_id`, `label` and the existing
  `usage` snapshot. Meters retain their provider source, observation time,
  duration, reset, percentage and reported credit balance. No missing number
  is estimated. The account namespace matches the ordinary account list.
- `GetAgentUsageSnapshot { session_id, agent_id }` returns
  `AgentUsageSnapshot { snapshot }`. Room membership and agent ownership are
  checked. `context` is absent until the current provider session reports used
  tokens; the limit is optional. `route` names the actual execution target and
  coordinating kernel when applicable. It never serializes remote admission
  credentials. Provider-run activity time is not a context observation time.

Codex supplies official app-server rate-limit/usage snapshots and token-usage
notifications. Claude supplies official rate-limit events and status-line
usage. Claude result `modelUsage[model].contextWindow` supplies a context limit
only for the observed main-agent model; aggregate model billing and subagent
input are not the main conversation context. OpenCode's official CLI stats provide local cost/tokens, while its Go
usage surface can report rolling limits. Local costs are not a subscription
balance. Missing windows or credits remain missing.

## Acceptance scenarios

These are required scenarios, not claims of acceptance. MP-10 real-path evidence
must identify the exact kernel, client, relay and official provider versions.

| ID | Required real behavior |
| --- | --- |
| provider-usage/USAGE-01 | The hosted System bar shows each linked account and only its reported 5-hour, weekly and credit meters, including real zero values, reset and observation times. No-number accounts show only their names. The menu is read-only and uses the prototype. |
| provider-usage/USAGE-02 | A real Codex gpt-6-luna low turn updates account usage from the official app-server and moves the selected agent's context ring using observed used/limit tokens. |
| provider-usage/USAGE-03 | A real Claude claude-haiku-5-5 turn exposes the account information actually reported by Claude, without estimated missing windows or context limits. |
| provider-usage/USAGE-04 | Selecting different agents/Rooms keeps context and execution route scoped to that agent; remote/slice execution names its true target via the coordinating kernel. Disconnects and late replies cannot retain a previous target's facts. |
| provider-usage/USAGE-05 | A real protocol-567 kernel omits the new provider buttons, context ring and routed label, sends no new requests and produces no compatibility errors. |
| provider-usage/USAGE-06 | A fresh normal-click/key flow on the real hosted stack is recorded at 1440x900 DPR1 and DPR2, with all eight prototype skins, relay recovery and a multi-hour stability session on the same exact heads; recordings are reviewed for missing/blank/stale/error content. |

Source, domain and component tests are supplementary regression evidence. A
passing test alone does not close a row or MP-10.
