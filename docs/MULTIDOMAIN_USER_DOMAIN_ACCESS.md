# User-domain access: the owner's agents need no grant

Owner decision, 2026-11-21 (repeated): agents never need a grant for the owner's
kernel browser, Computer or installed Apps. There is no grant object, no epoch,
no focus gating and no lifetime. This replaces the grant contract of local 432 and
the 2026-11-18 "load grants it" rule.

## What an agent gets

Every local agent of the owner, at any depth and in any turn, uses the owner's
kernel browser (every tab), Computer and App views as soon as the call is made.
`chariox.load_kernel_browser` and `chariox.load_kernel_computer` only list the
tools in the agent's catalog; they take no arguments, grant nothing and ask nobody.
A call is never refused for an expired, revoked, unfocused or stale grant, and
there is no delegation step between parents and children.

## What stays

- **Foreign owners.** A caller that is not the owner (or whose agent belongs to
  another owner) is refused; a foreign and a missing id look the same
  (`foreign_owner`).
- **Run fence.** The kernel keeps one cancellation fence per running agent. When
  the agent, its Room or its run ends, in-flight browser calls, App calls and
  subscriptions of that agent are cancelled. This is not a grant: it never refuses
  a live agent and carries no lifetime.
- **Secrets and money.** Vault-filled fields stay masked in what the model sees
  (frames, accessibility and DOM text). Payments, sudo, passkeys and person-presence
  steps keep their own cards.
- **Stale pages.** A document generation or page reference the caller holds can be
  out of date (`stale_epoch`, `stale_reference`); the caller observes again.
- **Person-only operations.** Human takeover, human-terminal display and similar
  operations refuse agent callers (`not_granted` is the code the controller uses
  for such a refused operation).

## Person cards with a live page

A card that shows a live browser page needs no grant to keep observing it, so a
kernel restart (hot reload or cold) keeps the same card; the watcher resumes at
recovery and the person finishes it as before. The restart ends the kernel's
browser; the watcher starts it again, the Tab comes back with a new document, and
a plain card (no awaited text, no verification) is bound again to the whole page of
that Tab, so the person sees the page fresh. A card that waits for page text or a
verification keeps closing when its document changes, because its proof belongs to
one document.

## No per-agent switches

The person has no per-agent switch for the owner's kernel browser, Computer or Apps.
The Room Computer switches (`grant_room_computer`, `revoke_grants`, `subscribe_grants`,
`list_grants`) and the `user_domain_grants_changed` snapshot were removed in local
1015. What stays: the explicit leased desktop grant (`grant_leased_computer`,
`revoke_leased_computer`), foreign-owner isolation, Vault masking and the stale-page
codes. A browser the person stops on purpose stays stopped: a person card's restart
recovery (`card_recovery`) starts only a browser the kernel's restart ended.

## Clients

The owner terminal's `/access` lists the kernel browser's tabs and gives or takes back a leased desktop; the web client has no Access view. The browser window and
App view projections no longer carry `kernel_id`, `kernel_name`,
`focused_agent_kernel_id` or `reachable_by_focused_agent`.
