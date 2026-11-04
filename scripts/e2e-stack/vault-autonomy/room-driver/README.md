# Room-agent account driver (MP-08 / MP-09 / MP-10 / MP-11)

Runs the account-signup autonomy rows (AU-03, AU-04, AU-08, AU-16) the way a user would: a real provider agent in a Room
creates the accounts itself with the drill Gmail identity, and hands one step to a person only when a service needs one
(CAPTCHA, phone, terms, a refusal). The person answers the card in the Room; the agent continues alone.

Environment: `ROOM_KERNEL_ROOT` (kernel dir with `ports.json` and `home/`, same host), `ROOM_SESSION`, `ROOM_DIR` (scratch outside
the repository), and for `viewer.mjs` also `ROOM_ORIGIN`, `ROOM_EVIDENCE`, `ROOM_OWNER_EMAIL`, `ROOM_OWNER_PASSWORD_FILE`, `ROOM_PLAYWRIGHT`.

- `setup-agent.mjs` links a provider profile directory the kernel user can read and spawns the agent.
- `driver.mjs` submits `ROOM_DIR/cmd/<agent-id>.<n>.prompt`, and writes `events.jsonl` (provider output and tool calls),
  `interactions.jsonl` (every Room card: appeared and gone with seconds open: the human-step timings) and
  `agentstate.jsonl` (agent and task states: Working, Waiting, Blocked).
- `front-page.mjs <url>` puts a page in the tab that is in front (the owner API cannot activate a tab) and lists the tabs; `stop-agent.mjs <agent-id> <attachment-id>` cancels an agent's running and queued prompts.
- `answer-card.mjs` lists or answers open cards. `brief.py` and `wait-event.sh` read the logs. `viewer.mjs` is the owner's real web view.

Learned with this rig: a blocking `request_popup` is cut by the provider after about 5 minutes (use `wait:false` and
`chariox.events.yield` for a person); an agent's shell can only write under the kernel user's `$HOME`; a value captured from
a page with `use_on` is browser-only, so use `use_binding` when a command or the HTTP API must use it.
