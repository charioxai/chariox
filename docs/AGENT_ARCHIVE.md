# Reversible agent archive (MP-08 / MP-09 / MP-10 / MP-11)

Miguel's 2026-10-27 correction supersedes the earlier Room proposal: archive one
agent inside a Room. The Room stays active and other agents are untouched.

## Existing path and shared protocol

Web's agent options already sends `ArchiveAgent { session_id, agent_id,
archived }` (minimum local592); it does not call DestroyAgent. The kernel already
persists the agent's archived flag, excludes it from freeform visibility, repairs
focus and fences stale prepared prompt admission under PromptStateOwner's lock.
Today it refuses running/queued work and leaves idle provider resources alive.
Reuse this request, identity, persistence and focus path. Replace the busy refusal
with bounded, acknowledged cleanup. Never remove/recreate the home agent.

Add `ListArchivedAgents { session_id }`; ordinary ListAgents omits archived agents.
GetSessionState keeps all agents and their archived flags for discovery/history.
Allocated protocol local636/peer239 covers that request and the additive
SubmitLeasedPrompt.provider_resume_state handoff. A fresh worker backing agent
receives the home thread before launch; an existing worker run retains its own
current resume state. New archive UI
requires 636; older kernels keep their existing controls/behavior, without a
fallback to destructive removal. All clients and remote kernels use this path.

## State and failure handling

Archive fences work under the prompt admission lock, then durably sets the
existing archived flag before asynchronous cleanup. No agent lock or durable
writer I/O runs while that admission lock is held. No new prompt or provider launch may pass afterward.
Cleanup operates only on the target. Success confirms it stopped; errors name
the failed resource and say to retry archive. The archived fence remains. Retrying
archive is safe; unarchive completes/rechecks pending cleanup before reopening.
Restart replay keeps the flag and identity. Unarchive returns the same agent to
idle, with no prompt/turn launched; the next owner prompt starts a fresh official
provider process using the saved thread where supported.

After worker cleanup is acknowledged, archive ends the target's home-only
provider-run projections and clears their routing and prompt activity, just as
destroy-agent cleanup does. Delayed snapshots of those runs remain Ended, and
unarchive cannot select a run from the destroyed lease.

Archive and unarchive leave output sequences and per-user seen receipts unchanged.
The archived flag and prompt settlements carry the lifecycle outcome; redundant
agent-output notices would create unread output that the Web cannot present.
An agent with acknowledged or no output stays read, and real unseen output stays
unread after unarchive, including repeated requests.

Only the agent owner may archive/unarchive, under existing authenticated owner
admission and Room membership. Room agents cannot archive/unarchive anyone,
including themselves or children, directly or through sudo. Collaborators retain
read visibility subject to existing Room policy; they gain no owner authority.

| Resource | Archive target agent | Unarchive |
| --- | --- | --- |
| Definition and identity | Keep ID, alias, owner, immutable parentage, model/profile, configuration and placement intent | Same object, idle |
| Provider thread | Keep provider session/thread ID and transcript cursors; terminate official provider process | Fresh process resumes saved thread on next prompt |
| History/worktree | Keep transcript, prompt outcomes, files, worktree and bindings | Readable throughout |
| Running/queued prompts | Cancel with archive reason, record terminal outcomes before success | Never replay cancelled prompts |
| Wakes/processes | Cancel target's timers and watched work; retain receipts and await physical stop | Owner must create new work |
| Goal/schedules | Cancel target's goal and legacy prompt schedules, retain outcomes in history | No automatic continuation or missed-interval replay |
| Interactions | Withdraw target's process-bound interactions | New run requests fresh decisions as needed |
| App grants | Keep bindings; suspend use while archived | Existing unexpired, non-revoked grants become usable |
| Terminals | Close target-owned provider/Room terminals; keep history | Fresh terminals on demand |
| Worker/remote lease | Release with acknowledgement; retain old binding only as suspended placement intent | Reacquire through existing binding refresh, fail clearly if worker unavailable |
| Environment/slice | Keep Room environment, profile, slice and worktree; remove target's live actors | Reconcile actors without restarting other agents |
| Membership | Keep Room owner/members, agent owner and collaboration visibility | Unchanged |

Codex uses its saved thread through the official app-server/CLI resume path;
Claude Code uses its saved session through the official resume option; OpenCode
uses its saved session through the official server/session path. The original
profile and provider-owned storage must remain accessible and still contain that
thread: existing provider-specific recovery applies if it is missing (OpenCode may create a replacement native session).
An external endpoint must reconnect itself. Live drills establish provider resume;
retaining an ID alone does not establish it.

## Edge cases

- Default/selected/only agent: repair focus to the first visible remaining agent,
  or none. The Room stays active and readable; its Prompt bar has no agent route
  until the user selects/unarchives/spawns one. Never create a replacement agent.
- Children/collaboration: preserve identities and running children. Cancel the
  target's participation/subscriptions, while children and shared workflow runs
  continue. Their results remain in history/inbox for later inspection. Do not
  propagate archive into child cancellation. Idle parentage is kept.
- Active or queued workflow node: refuse before changing state and ask the owner to pause
  or rebind that invocation; never stop another node implicitly.
- Workflow binding: retain definitions. A future run targeting an archived agent
  fails clearly with an unarchive/rebind instruction. Do not steal workflow claims.
- Leased agent: release only its lease and terminals; other agents' leases and
  the Room environment stay. Refresh placement through ordinary worker admission
  on unarchive, without a local fallback. The home history remains authoritative.
- Notification/mail: history and existing notifications remain readable; messages
  may be retained for the stopped agent but cannot wake it while archived.
- Usage: keep historical meters; stopped provider/lease resources stop accruing
  runtime usage. Cloud managed-machine shutdown policy is unchanged.
- EndSession/DestroyAgent: keep their destructive semantics and existing legacy
  recovery. This feature does not reconstruct previously removed identities.

## UI proposal — approval required

Use the existing OS prototype components and byte-identical CSS. At the BOTTOM of
the Room's agents side panel, add `View archive`; it opens an existing popover/menu
with archived agent rows and an `Unarchive` action on each. Reuse the existing
empty-state row for `No archived agents`. The agent's own options menu says
`Archive`. Archived agents disappear from the active panel, Prompt bar route and
agent tabs. After archiving the selected agent, focus the next visible agent, or
show the existing no-agent state if it was the last. No Archived Rooms destination,
Room footer Archive/Unarchive or archived-Room view is proposed.

Render labelled PROPOSAL mocks at DPR1, desktop1440×900 and phone400×900, for the
panel/footer row, populated dropdown, empty dropdown, agent Archive menu and
post-selected-agent archive. Production UI waits for Miguel's approval.

## Required acceptance

Real provider running + queued prompt; archive stops only target with cancelled
outcomes, closes its terminals/wakes/lease, retains grants and other agents' work.
Restart, list archive, unarchive same identity/history/thread, then send one prompt
and prove provider resume. Check only-agent focus, collaborator and agent/sudo
negative admission, pending-cleanup retry/restart, stale admission races, workflow
refusal and independent collaboration cancellation, schedule cancellation, older-kernel feature gating,
and legacy EndSession unchanged. Proposal mocks are not UI acceptance.

### Round r1 validation allocation

Codex and Claude require live official-provider archive/restart/resume drills.
Per the coordinator's 2026-10-27 07:09Z answer, live OpenCode is NR this round:
the acct-686 scope was deleted during disk cleanup and no OpenCode login exists
on b3. Do not wait for or request another login. Cover archive, durable restart,
unarchive and fresh native-runtime resume with the fake-server contract fixture;
the coordinator's later Chariox account transfer is not an archive r1 blocker.
