# MP-08 / MP-10 / MP-11: user-domain access

Owner decision, 2026-10-05. Local protocol **432**, relay peer **78**. This
contract supersedes the focus-revocation text in earlier multidomain receipts.
The kernel owns it equally on ordinary and managed placements.

Focus grants access. Changing focus keeps the previous agent's grant and gives
the new agent its own grant. Browser and notes loading remain independent.
Each grant retains only stable resources the agent used while focused. A
retained agent can observe and operate its claimed resources; opening a tab,
starting/stopping the user browser, loading another capability or claiming a
new tab/note requires focus. Generation, document, owner and provider-run
checks still apply. Listing metadata does not claim every resource listed.
Retained browser/notes inventories show only claimed resources.

The kernel reads its existing active turns, prompt ownership/backlog, scheduled
agent prompts, interactions and pending MCP/provider continuations. A provider
turn includes waits on harness subprocesses, tools and subagents. Those waits
retain access. A live but idle provider process does not retain access forever.
Only a fully idle agent starts the expiry window. The default is 30 minutes;
`CHARIOX_USER_DOMAIN_IDLE_TIMEOUT_SECONDS` accepts a positive integer override.
Idle duration uses a monotonic clock. Session end, agent destruction, placement
retirement and explicit revoke invalidate admission immediately. Kernel restart
does not restore runtime authority from browser profiles.

Authenticated terminals use `KernelBrowser` commands `list_grants`,
`subscribe_grants { after, wait_ms }` (maximum 25 seconds) and
`revoke_grants { agent_id }` (`null` revokes all of that owner's holders).
These commands do not need a healthy browser or unlocked Vault and bypass
cached observations. Their `user_domain_grants_changed` snapshot includes a
cursor, agent/session/kernel, resources, grant time, focus and expiry rule.
Non-focused resource use advances the cursor and publishes a value-free notice.
Clients keep the last cursor and resubscribe; responses never include resource
contents, credential values or another owner's grants.

Revocation cancels the admission epoch first, retires actor presence and
refuses in-flight results/commits at the next authority check. The same epoch
fences idle subscriptions; an internal controller cleanup closes their streams.
It cannot undo physical input already delivered. Vault fills always require
focus. Native input classifies protected/password/OTP/payment fields, opaque
frames/shadow hosts and payment/critical approval controls before dispatch.
Unknown classification fails closed. Sensitive input requires focus throughout
the operation; retained routine input rechecks protection before physical events.
Critical App effects continue through kernel-owned human validation/passkeys.

There is no cross-kernel control. Browser windows expose `kernel_id`,
`kernel_name`, `focused_agent_kernel_id` and `reachable_by_focused_agent`; App
views expose the same fields under `access`. A remote/leased runtime MCP call
fails with the execution/window kernels and asks the agent to request focus on
the window's kernel. Shared client `userDomainWindowBadge` supplies:
"Your focused agent can't control this window — focus an agent on kernel <name>".
Only clients consuming these new commands/projections need minimum 432; existing
browser/App/notes/capture behavior keeps minimum 427.

App and capture IDs have explicit resource kinds in the shared grant contract.
Browser-hosted App observations use the browser authority; browser mutations
still cannot impersonate a human App channel. The separately proposed App-agent
MCP adapter remains unimplemented in this source. Visible-region capture remains
human-only and becomes ordinary prompt input only when the user sends it. This
change creates no agent capture API, approval-answer path or cross-kernel bridge.

MP-10 evidence must distinguish unit/fixture checks, native Linux browser
execution, official-provider subagent waits, clients, and fresh-machine parity.
None alone closes an MP item.

## Client consumers (mdgrants)

The owner TUI provides `/access` (or `/access list`) and
`/access revoke <agent-id>|all`, through the ordinary owner-terminal
`KernelBrowser` path. Its live grant feed prints a compact retained-use notice.
The web Access view lives inside the existing Browser dialog; no new rail item
is added. It lists holder, session, kernel, touched resource IDs, observed
retained-use time and the kernel idle deadline, with per-holder and all-holder
revocation. Both consumers share the cursor projection and notice formatting.
Only these grant consumers and cross-kernel badges require 432; existing
multidomain browser/App/notes/capture minima remain 427.

Protocol 432 does not expose a per-holder last-use timestamp. Clients label the
latest notice they observed as **last observed retained use**, show **Not
observed** otherwise, and do not infer use from grant creation, idle transitions
or focus. The live snapshot carries only the latest owner notice, so this is
not a complete activity audit. A complete last-use projection requires a future
coordinator-allocated protocol change and updated wire snapshots.

Reproduce the separate-process client drill after building the shared client
and the kernel with `slot-run`:

```sh
CARGO_TARGET_DIR=/w/cx-mdgrants/target CARGO_PROFILE_DEV_DEBUG=0 \
  slot-run cargo build -p chariox-kernel --bin chariox-kernel
node apps/cli/scripts/user-domain-access-live-drill.mjs /w/evidence/mdgrants
```

It creates an explicit disposable `CHARIOX_HOME` under the evidence state
root, uses owner WebSocket controls and provider-bound MCP calls against the
production kernel, holds a dev-stub turn across focus change, claims a panel
note, verifies retained use and revocation, and cleans its owned process group
and state. It does not claim official-provider, native-browser, hosted relay or
fresh-machine acceptance.
