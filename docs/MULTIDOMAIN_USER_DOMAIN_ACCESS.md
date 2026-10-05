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
Unknown classification fails closed. Enter resolves the native form's first
associated submit control, including external controls; buttonless or unnamed
activation requires focus. Chromium's built-in control shadow trees are not
opaque page content; page-created and unknown shadow roots remain protected.
Focused physical input carries live focus authority for the whole operation,
allowing routine-to-sensitive transitions such as Tab onto an approval button.
Focus loss cancels that operation without revoking its retained resource grant;
the agent may retry routine input under retained authority. Retained routine
input rechecks protection before physical events. Tab's non-activating paired
release checks document and cancellation authority without reclassifying the
newly focused button as an activation.
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

MP-08 / MP-10 / MP-11 PR #880 regressions: the opt-in
`kernel-browser-input.browser-test.mjs` exercises real native form submission
and paired navigation through the production controller. The ignored Rust
`runtime::kernel_browser_host::native_input_tests` drills additionally prove
kernel-focused navigation preserves tab/document generation and idle streams,
and retained Enter refuses payment. These credential-free native checks do not
establish official-provider, client, hosted, or fresh-machine acceptance.
