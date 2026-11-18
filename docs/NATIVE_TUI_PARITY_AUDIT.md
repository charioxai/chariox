# Native provider terminals as agent panes: audit

Status: audit and proposal, 2026-11-18. No product code exists for it. Nothing here is decided until the owner accepts the validation cells in section 9 and answers the open questions in section 10. Screens, logs and scratch sources are kept outside the repository under `/root/.codex/evidence/orch12/nativetui-audit/`.

The goal, in the owner's words: `chariox claude`, `chariox codex` and `chariox opencode` should behave like an agent pane in a Room of the web client. The launch choices (Room or new Room, Project, slice or not, model and so on) come as command-line arguments. The worry is the actions a person can take and the information they see inside the provider's own terminal UI. Chariox's own terminal UI (`apps/cli`'s Ink client) is out of scope: this is only about the providers' terminal UIs run through Chariox.

Every decision this document proposes names the principle behind it:

- **P1, simplest for the person:** fewest steps, no settings, no error text, no connection details.
- **P2, agents do it themselves:** a person step exists only for secrets, money, risky irreversible actions or another party's consent.
- **P3, familiar patterns:** every visible element follows the Web terminal and its prototype; inside a provider's terminal it follows that provider's own dialogs.

## 1. Summary

1. Today a native terminal is a faithful provider UI with a kernel behind it, not an agent pane. Prompts, replies, stop and the provider's own permission prompt reach the kernel. Nothing else from the Room reaches the terminal: no kernel cards, no stripes, no Room or agent identity after the launch banner, no Room commands, no notices.
2. An agent that asks the person a question through a kernel card is stuck in a native terminal until the person opens another client. Live on Claude Code: the terminal showed `Calling chariox · 11s…` and nothing else while the card `Audit card` waited in the kernel.
3. All three providers have an official surface that can show a kernel card as the provider's own dialog, with no model turn and no screen scraping. Proven in scratch runs on the real terminal UIs for Claude Code (MCP elicitation, over stdio and HTTP) and OpenCode (question events); read in source for Codex (app-server server requests).
4. The recommended shape is one kernel-owned "pane feed" per agent (identity, settings, state, cards with a kernel-decided "can be answered in a text terminal" flag and a link, stripes, usage, notices) and three thin adapters that only translate that feed into provider-native elements and translate answers back into `RespondToInteraction`. About 9 lane-weeks, of which three adapters can run in parallel.
5. The audit found ten defects on today's path (section 3.6). Three block ordinary use: `chariox codex` defaults to a model Codex no longer serves, `chariox opencode` cannot send a prompt without an OpenCode account even on a free model and shows a bare `502`, and denying a permission in Claude's own dialog leaves the kernel card and the prompt open.

## 2. How this was checked

Versions: Chariox OSS `main` at `d687f3b911` (kernel built from it, protocol 1011), Cloud web client at `d8b8df0d49`, Claude Code 2.1.296, Codex CLI 0.159.3 (source tag `rust-v0.159.3`), OpenCode 1.19.0 (source tag `v1.19.0`).

Every claim carries one of these labels:

| Label | Meaning |
| --- | --- |
| **live** | Seen on the real provider terminal UI launched by `chariox <provider>` against a kernel of `d687f3b911` started only for this audit, with a second kernel client standing in for the web pane. |
| **scratch** | Seen on the real provider terminal UI against a small scratch server written for the audit (an MCP server, an OpenCode pass-through proxy). Not through Chariox. It proves what the provider's UI does with a message, not that Chariox sends it. |
| **source** | Read in the provider's open source at the tag above. Not run. |
| **docs** | Read in the provider's official documentation on 2026-11-18. Not run. |
| **code** | Read in Chariox code. Not run in this audit. |
| **not run** | Could not be run; the reason is given. |

Limits of the live runs:

- Claude Code: Haiku 5.5, low effort, about ten short turns.
- Codex: one turn was allowed to run and it failed (finding F1). The account's weekly meter was at 87%, so no further Codex turn was run. Codex approvals and cards are **source** and **code**.
- OpenCode: the launch is live. No turn ran, because the kernel refuses a prompt without an authenticated OpenCode account and this audit had none (finding F2). OpenCode prompt, permission and abort handling are **code**; the closing mechanisms are **scratch**.
- The launchers' banner was captured from standard error, so how long it stays on screen was not observed.

## 3. Status today

### 3.1 Launch arguments

All three launchers take: the kernel to use (`--socket`, `--kernel-url`, `--kernel-port`, relay and target-kernel options), `--workspace`, `--worktree`, `--machine`, `--slice`, `--alias` (Room), `--agent-alias`, `--model`, `--mode build|plan`, `--permissions required|yolo`, `--grant-mcp`, `--grant-skill` and a positional Room reference to reopen a Room. Claude and Codex also take `--effort` and `--initial-prompt`. (**code**: `apps/cli/src/native-tui/{claude,codex,opencode}.ts`.)

| Launch choice in the web client | Native launchers |
| --- | --- |
| Existing Room | Present: positional Room reference. **live** on Codex. |
| New Room | Partial: a launch without a reference creates one; there is no explicit "new Room" word. |
| Project | Missing. The kernel's create-Room request accepts a Project selection; no launcher passes one. **code** |
| Provider account | Missing. The kernel default account is used. **live**: the audit had to change the kernel default to pick an account. |
| Environment | Missing. |
| Slice, machine, worktree | Present. Slice and remote machine were not run here; they are open acceptance rows (section 11). |
| Model, effort, mode, permissions | Present, with stale defaults: Claude `claude-sonnet-4-6`, Codex `gpt-5.4-mini` (no longer served, F1), OpenCode `default`. Codex and OpenCode default to `yolo`, Claude to `required`. **code** |

### 3.2 What the launchers show

Before the provider UI starts, the launcher prints a banner on standard error: Room id and alias, agent id and alias, home kernel, owner, worktree, placement, extensions, provider run and the provider's endpoints. **live**, all three. After that the screen belongs to the provider. Nothing from Chariox is drawn again.

The banner's last line says `prompt policy: native prompts pass through; Chariox observes the session`. That is not what the code does for Codex and OpenCode: their proxies stop the prompt and submit it through the kernel (**code**; **live** on both, see F1 and F2). Finding F9.

### 3.3 Claude Code

The kernel owns the pseudo-terminal and streams it to the launcher. It writes a settings file with hooks for SessionStart, UserPromptSubmit, Stop, StopFailure, SessionEnd and PermissionRequest, and a `statusLine` command that only records usage and prints nothing. The Chariox MCP server is attached. (**code**: `apps/kernel/src/provider/claude/native_tui.rs`.)

| Behaviour | Result | Label |
| --- | --- | --- |
| Launch, banner, provider run | Works. | live |
| First launch in a new folder | Claude's own "trust this folder" dialog, default choice "No, exit". No kernel card exists for it, so another client sees an agent that never starts. | live |
| Prompt typed in the terminal | Reply shown; the kernel records the turn, its outcome and duration. | live |
| Prompt sent from another client | Typed into the terminal by the kernel, reply shown in both places. | live |
| Permission prompt, `--permissions required` | Claude's own dialog and one kernel card are open together. | live |
| Answer "Yes" in the terminal | Tool runs; the kernel card closes within about 6 s. | live |
| Answer "Allow once" on the other client | Claude's dialog closes and the tool runs. First answer wins. | live |
| Answer "No" in the terminal | Defect F3. | live |
| Esc | Cancels the active prompt in the kernel. | live |
| Agent raises a kernel choice card | The terminal shows only `Calling chariox · 11s…`. The card is answerable only from another client. | live |
| Room command typed (`/notifications`) | `Unknown command: /notifications`. | live |
| `/model sonnet` in the terminal | Claude switches and saves it as its default; the kernel still says `haiku`. Defect F6. | live |
| Footer | Claude's own (`manual mode on`). No Room, agent, model as the kernel sees it, cards, stripes or usage. | live |
| Exit | Claude prints `claude --resume <id>`. Defect F8. | live |
| Reopen a Room | Starts a new top-level agent. All three launchers resolve the Room and call `spawnNativeAgent` (`SpawnAgent`); a native launch never attaches to an existing agent or provider run (`docs/PROTOCOL.md` 3.3.2). The closed agent's turns are not shown. | code |

### 3.4 Codex

The launcher starts `codex --remote` against its own proxy in front of the kernel's Codex app-server. The proxy stops `turn/start`, `turn/interrupt`, `turn/steer` and compaction and sends them through the kernel; kernel output is projected into a display thread. Only a permission card of this agent's own running prompt is projected, as a synthetic `item/commandExecution/requestApproval`. Kernel notices are not polled. (**code**: `apps/cli/src/native-tui/codex-proxy.ts`, `codex-kernel-permission-projection.ts`.)

| Behaviour | Result | Label |
| --- | --- | --- |
| Launch, banner, provider run | Works. | live |
| First launch in a new folder | Codex's own folder-trust dialog; no kernel card. | live |
| Default model | Defect F1: Codex shows "GPT-5.4 Mini is no longer available", the turn fails in the kernel and the terminal sits in "Reconnecting to app-server…". | live |
| Footer and `/status` | Codex's own, and wrong: the footer said `GPT-6-Luna medium`, later `gpt-5.4-mini medium`; `/status` said "Read Only (Ask for approval)". The kernel run was `gpt-5.4-mini`, effort low, workspace-write. Defect F6. | live |
| Reopen a Room | Opens, as a new agent (same launcher path as Claude); the failed prompt was not shown in the reopened transcript. | live |
| Room command typed (`/notifications`) | `Unrecognized command '/notifications'`. It never leaves the terminal. | live |
| Exit | Codex prints `codex --remote ws://127.0.0.1:<port>/ resume <id>`, a proxy that no longer exists. Defect F8. | live |
| Prompt, reply, steer, stop | Through the kernel. | code, fixture tests |
| Permission prompt | Kernel card shown as Codex's own approval overlay; the answer goes to the kernel. Only for this agent's own running prompt. | code, fixture tests |
| Kernel choice cards | Not projected. | code |

### 3.5 OpenCode

The launcher starts `opencode attach` against its own HTTP proxy in front of a kernel-managed `opencode serve`. The proxy sends prompts through the kernel, sends abort as a kernel cancel, routes permission replies to the kernel card, filters the event stream and applies a model or variant chosen in the terminal to the kernel run. It has no handling for questions or toasts. (**code**: `apps/cli/src/native-tui/opencode-proxy.ts`.)

| Behaviour | Result | Label |
| --- | --- | --- |
| Launch, banner, provider run, attach | Works; the terminal shows `Build · Big Pickle`. | live |
| Prompt typed in the terminal | Defect F2: "Failed to send prompt … 502 Bad Gateway". The reason (no authenticated OpenCode account) is only in the proxy's response body. | live |
| Exit | OpenCode prints `opencode -s <id>`. Defect F8. | live |
| Prompt, permission, abort, model switch | As described above. | code; not run (F2) |
| Kernel choice cards | Not projected. | code |

### 3.6 Defects found on today's path

These are product defects for later lanes. None is fixed here.

| # | Defect | Label |
| --- | --- | --- |
| F1 | `chariox codex` defaults to `gpt-5.4-mini`. Codex CLI 0.159.3 no longer serves it and names GPT-6 Luna. The kernel turn fails with "The provider could not complete this request. Check the provider account and model settings", and the terminal shows only "Connection lost. Attempting to reconnect…" for as long as it stays open. | live |
| F2 | `chariox opencode` cannot send a prompt without an authenticated OpenCode account, even on a free model that plain `opencode` runs with no account. The terminal shows `502 Bad Gateway` and no reason or next step. | live |
| F3 | Claude: answering "No" in Claude's own permission dialog leaves the kernel card open until its 300 s timeout and the prompt "running" until someone presses Esc (320 s in the run). In that state two prompts sent from another client were accepted, never reached the terminal and were cancelled after 30 s with "Needs your help". Likely cause (**code**): the kernel's prompt injection returns early while a `permission:` marker is set, and Claude Code runs no hook when the person denies in its dialog. | live |
| F4 | Claude's and Codex's first-run dialogs (folder trust, model retirement) have no kernel card, so another client sees an agent that silently does not start. Already parked as MP-10-FIRST-MANAGED-CLAUDE-DPR1. | live |
| F5 | A kernel choice card raised by the agent is invisible in every native terminal. | live (Claude), code (Codex, OpenCode) |
| F6 | The provider's own model, effort and permission display can differ from the kernel's record, and a switch made in Claude's terminal is not reported to the kernel. | live |
| F7 | Room commands typed in the terminal are rejected by the provider as unknown commands. | live (Claude, Codex) |
| F8 | On exit every provider prints its own resume command, which bypasses Chariox or points at a proxy that is gone. The Chariox command to reopen the Room is not printed. | live |
| F9 | The banner's "native prompts pass through" line is wrong for Codex and OpenCode. | live, code |
| F10 | Seen once, not diagnosed: after a prompt from the other client, Claude's Stop hook ran for 94 s, and the kernel log shows it detached both launchers' attachments as stale at the end of that window. | live |

Claude Code's permission dialog also offers "Yes, and always allow access to … from this project" and "Yes, and switch to auto mode". Both change Claude's own rules without the kernel knowing (**scratch**). See open question Q3.

## 4. Gap table against a web Room agent pane

The feature list comes from the web pane source in Cloud at `d8b8df0d49`, `apps/web/src/terminal-os/`: `agent-pane.tsx`, `agent-config.tsx`, `agent-path.tsx`, `room-transcript.tsx`, `pane-turn-info.tsx`, `pane-tool.tsx`, `pane-plan.tsx`, `pane-visual.tsx`, `pane-references.tsx`, `interaction-cards.tsx`, `interaction-browser-region.tsx`, `payment-approval-card.tsx`, `app-confirmation-card.tsx`, `vault-unlock-popup.tsx`, `interaction-stripes.tsx`, `pending-prompts.tsx`, `sub-agents-stripe.tsx`, `usage-limit-stripe.tsx`, `prompt-bar.tsx`, `prompt-attachments.ts`, `prompt-settings.tsx`, `provider-usage.tsx`, `pane-overview.tsx`, `room-agent-rows.tsx`, `use-startup-target.ts`.

"Present" means the person gets the same result in the terminal, even if the provider draws it in its own way. "Partial" means some of it, or the provider's own version that the kernel does not know about. Labels as in section 2; a cell without a label is **code**.

| # | Web pane feature | Claude Code | Codex | OpenCode |
| --- | --- | --- | --- | --- |
| **Header** | | | | |
| 1 | Room name, agent name, sub-agent path | Partial: launch banner only (live) | Partial: launch banner only (live) | Partial: launch banner only (live) |
| 2 | Settings shown: provider, account, model, effort, mode, permissions, worktree, machine | Partial: Claude's own model and mode lines; no account or placement (live) | Partial and wrong, F6 (live) | Partial: agent and model name (live) |
| 3 | Switch model, effort, mode, permissions, account | Partial: Claude's own `/model`, not told to the kernel (live) | Missing: a choice in the terminal does not change the kernel run | Partial: model and variant are applied to the kernel run; not run |
| 4 | Status dot, running line ("Waiting for your approval") | Partial: Claude's own spinner (live) | Partial: Codex's own | Partial: OpenCode's own |
| 5 | Archive, delete, carry, hide, find | Missing (find is the terminal's own search) | Missing | Missing |
| **Transcript** | | | | |
| 6 | Streaming reply, reasoning, tool rows | Present (live) | Present, projected thread (fixture tests) | Present; not run |
| 7 | Prompt from another client appears | Present (live) | Present | Present; not run |
| 8 | Earlier turns on reopen, paging | Missing: reopen starts a new agent (code) | Missing: reopen starts a new agent (code); reopened empty after a failed turn (live) | Missing: reopen starts a new agent (code) |
| 9 | Edit and resend, Continue | Partial: provider's own | Partial: provider's own | Partial: provider's own |
| 10 | Plan card with Implement | Partial: Claude's own plan approval | Partial | Partial |
| 11 | Visuals, file references that open the Files view, mentions | Missing | Missing | Missing |
| 12 | Wait-event rows with cancel, Room setup rows, send failures | Missing | Missing; send failure shown as "Connection lost" (live) | Missing; send failure shown as `502` (live) |
| **Cards** | | | | |
| 13 | Provider permission prompt, first answer wins | Present with defect F3 (live) | Present for the agent's own running prompt (fixture tests) | Present; not run |
| 14 | Kernel choice cards: agent questions, decisions, blocked-task resume | Missing (live) | Missing | Missing |
| 15 | Sub-agent cards shown in the parent pane with the asker's name | Missing | Missing | Missing |
| 16 | Vault unlock with durations; typed secret reply | Missing | Missing | Missing |
| 17 | Browser hand-off, browser region, "Show page" | Missing | Missing | Missing |
| 18 | Payment approval, App confirmation, file pick and grant, provider sign-in, `/sudo`, access grants, passkey choices | Missing | Missing | Missing |
| 19 | Provider's own first-run dialogs as cards (reverse gap, F4) | Terminal only (live) | Terminal only (live) | Not seen |
| **Stripes** | | | | |
| 20 | Queued prompts: edit, steer, remove | Partial: Claude's own queue for text typed there; the kernel queue is not shown | Partial: same | Partial: same |
| 21 | Wakes (`/wait-in`, `/wait-every`), triggers, `/goal`, task stopped, blocked wait, workflow run, sudo window | Missing | Missing | Missing |
| 22 | Sub-agents stripe, usage-limit stripe | Missing | Partial: Codex's own limit warning (live) | Missing |
| **Prompt bar** | | | | |
| 23 | Send, queue, Stop | Present (live) | Present | Present; not run |
| 24 | Attachments, drag and drop, pasted images | Partial: files named with `@` in the typed prompt are carried to the kernel | Partial: images in the provider's prompt are forwarded | Partial: attachments in the provider's prompt are carried to the kernel; not run |
| 25 | Slash menu with Room commands (39 top-level commands in the kernel catalog) | Missing, F7 (live) | Missing, F7 (live) | Missing |
| 26 | Capture region, picture editor, dictation, drafts | Missing; not a terminal feature | Missing | Missing |
| **Information** | | | | |
| 27 | Provider usage | Partial: Claude's own `/usage`; the kernel records usage silently | Partial: Codex's own `/status` (live) | Partial |
| 28 | Context meter | Not in the web agent pane either; each provider shows its own | Same | Same |
| 29 | Notifications, completion announcement, inbox | Missing; provider's own bell only | Missing | Missing |
| 30 | Room and agent switching, pane overview, collaborators | Missing: one terminal is one agent | Missing | Missing |
| 31 | Browser, Terminal and Files views | Missing | Missing | Missing |

## 5. What each provider offers

Runtime rule 2 limits the tools to the provider's official ones: its CLI, local server and documented hooks. Everything below is one of those. No SDK is used.

### 5.1 Claude Code 2.1.296

| Surface | What it does | Label |
| --- | --- | --- |
| MCP elicitation from the attached Chariox MCP server | A dialog "MCP server "chariox" requests your input" opens in the terminal, also while the agent is idle, with no model turn. A form with no fields is the message plus Accept and Decline (one key). One enum field is a list of choices (about four keys). A text field takes a typed reply. URL mode shows a link with Accept and Decline. The server's `notifications/cancelled` closes the dialog. Works over stdio and over the HTTP transport the kernel uses. A dialog sent while Claude's permission dialog is open waits behind it. | scratch |
| `statusLine` command | Up to several lines with colour and clickable links, refreshed on a timer while idle (`refreshInterval`, at least 1 s). Its input has the model, effort, cost, context window, rate limits and session. Display only; hidden while a dialog is open. | scratch |
| PermissionRequest hook | Runs while Claude's own dialog is open. A hook answer closes the dialog; a terminal answer wins if it comes first. After a terminal "Yes" PostToolUse runs; after a terminal "No" the hook process is ended and no hook event runs. The race is observed behaviour, not a documented contract. | scratch, live |
| UserPromptExpansion hook plus a command file | A typed `/chariox-cards` is handled by the hook with no model turn. The terminal shows "UserPromptExpansion operation blocked by hook:" followed by the hook's text. That frame wording cannot be changed. | scratch |
| PreModelSwitch hook | Runs on `/model` with the old and new model, before Claude's confirm step. | scratch |
| Notification, Elicitation, ElicitationResult hooks | Run for permission prompts, elicitation dialogs and their answers. | scratch |
| Hook output `systemMessage`, `terminalSequence` | A line for the person; terminal title, bell and desktop notification sequences. Only when a hook runs. | docs |
| Plugins (`--plugin-dir`) | Package commands, hooks, MCP servers and output styles as one unit. | docs |
| Channels | Would push messages and relay permissions, but custom channels need a development flag and a claude.ai sign-in. Not usable for a product. | docs |

### 5.2 Codex CLI 0.159.3

| Surface | What it does | Label |
| --- | --- | --- |
| App-server server requests through the launcher's proxy | The terminal UI handles `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, `item/tool/requestUserInput` (questions with options and notes; marked experimental) and `mcpServer/elicitation/request` (form mode only, with choice fields and masked secret text fields; URL mode is not drawn). | source: `codex-rs/tui/src/app/app_server_requests.rs`, `bottom_pane/request_user_input/`, `bottom_pane/mcp_server_elicitation.rs` |
| `serverRequest/resolved` | Tells the terminal a request was answered elsewhere. | docs |
| Notifications `warning`, `thread/name/updated`, `thread/goal/updated`, `account/rateLimits/updated`, `thread/tokenUsage/updated` | A warning line; the thread's name; Codex's own goal and usage displays. | docs |
| `tui.status_line`, `tui.terminal_title` | Built-in items only. There is no custom command. | docs |
| Hooks, `notify` | Hooks must be trusted by the person through `/hooks` unless managed; `notify` runs a program on turn end. | docs |
| `!command` shell escape | Runs a shell command from the prompt box. | docs |

### 5.3 OpenCode 1.19.0

| Surface | What it does | Label |
| --- | --- | --- |
| Question events on the event stream the terminal reads (`/global/event`) | A synthetic `question.asked` opens OpenCode's own question dialog while idle: numbered options, "Type your own answer", Esc to dismiss. The answer arrives at the proxy as `POST /question/<id>/reply`. A synthetic `question.replied` closes the dialog. | scratch |
| `permission.asked`, `/permission/<id>/reply` | OpenCode's permission dialog. Already used by the launcher's proxy. | code |
| `/tui/show-toast`, `tui.toast.show` | A toast in the terminal. | scratch |
| `/tui/append-prompt`, `/tui/execute-command`, `/tui/open-models` | Drive the prompt box and built-in commands. | docs |
| TUI plugins (`tui.json`) | Register slash commands, dialogs, toasts and content for slots such as `home_footer`, `session_prompt_right` and `sidebar_content`. New in this release line. | source: `packages/plugin/src/tui.ts` |
| Server plugins | Hooks on events, tools and permissions. | docs |

## 6. How to close each gap

One row per group of gaps from section 4. "Link" means the fallback of section 6.1.

| Gap (rows) | Claude Code | Codex | OpenCode | Hard limit |
| --- | --- | --- | --- | --- |
| Identity and settings (1, 2, 4) | Kernel-rendered `statusLine`: Room, agent path, model, effort, mode, permissions, placement, as the kernel records them. | `thread/name/updated` carries "Room ▸ agent". Terminal title written by the launcher. | TUI plugin footer slot. Terminal title written by the launcher. | Codex has no custom status line. Claude's is hidden while a dialog is open. |
| Settings switches (3) | PreModelSwitch and the `permission_mode` field of every hook report the change to the kernel; the kernel's refusal is shown by the hook. Account and placement changes need a relaunch. | The proxy applies the model and effort of the terminal's next `turn/start` to the kernel run, as OpenCode's proxy does today. | Already applied for model and variant. | A provider process keeps its account and placement until it is restarted. |
| Provider permission prompt (13) | Keep Claude's dialog as the terminal face of the one kernel card. Add the missing close-out: a terminal answer settles the kernel card at once (PostToolUse for "Yes"; for "No" the interruption record in Claude's transcript, which the kernel already reads). | Project every permission card shown in this agent's pane, not only its own running prompt. | Same as today. | Claude's race between hook and dialog is undocumented. |
| Kernel choice cards (14, 15, 16) | MCP elicitation from the Chariox MCP server: no-field form for two choices, one enum field for more, a text field for a typed reply (including a secret the person types and a Vault passphrase), the asker's name in the message. `notifications/cancelled` when answered elsewhere. | `mcpServer/elicitation/request` in form mode, or `item/tool/requestUserInput`; `serverRequest/resolved` when answered elsewhere. | Synthetic `question.asked`; `question.replied` when answered elsewhere. | Dialog titles are the provider's ("MCP server "chariox" requests your input"). Whether Codex draws a request with no running turn is not verified. |
| Cards a text terminal cannot answer (17, 18) | URL-mode elicitation with the link. | A `warning` line with the link (Codex does not draw URL mode). | Question dialog whose only option is "Open in Chariox", or a toast with the link. | A page, a passkey, a payment sheet and a file picker cannot be drawn in a provider's terminal. The kernel never puts a Vault value into a provider's form; what the person types there is theirs (AGENTS.md, secrets). |
| First-run dialogs (19) | The kernel reports "waiting for you in the terminal" in the pane feed so other clients say where the person must look. | Same. | Same. | These dialogs exist before any hook or server is attached. |
| Stripes (20, 21, 22) | Second and third `statusLine` lines: cards waiting, queued prompts, wakes, goal, sub-agents, usage limit. | One `warning` line when a stripe appears or changes. Details through the agent or the link. | TUI plugin sidebar slot. | Codex cannot keep a persistent line. |
| Room commands (25) | One command, `/chariox <Room command>`, handled by the UserPromptExpansion hook with the same kernel executor agents use (`chariox.room.run_command`). No model turn. | Ask the agent (it runs the command with `chariox.room.run_command`), or `!chariox <Room command>` through the shell escape. | TUI plugin slash commands. | Claude's "blocked by hook" frame wording. Codex rejects unknown slash commands before they leave the terminal (live). |
| Notices and completion (29) | `statusLine` within its refresh interval; `systemMessage` on the next hook; terminal title, bell and desktop notification from the launcher. | `warning`; terminal title, bell and desktop notification from the launcher. | Toast; same from the launcher. | None. |
| Usage (27) | Already in the status line input; print it. | Codex's own display, fed by `account/rateLimits/updated`. | Plugin footer. | None. |
| Resume an existing agent (8) and exit hint (F8) | A launcher option `--agent <name>` attaches the terminal to an existing agent of the Room: the kernel resumes that agent's own provider session (Claude `--resume` of the agent's session, run by the kernel) instead of `SpawnAgent`. The launcher prints `chariox claude <Room> --agent <name>` after the provider exits. | Same, through the agent's app-server thread. | Same, through the agent's OpenCode session. | The provider's own hint cannot be removed, only followed by the right one. Printing the hint without the resume path would open a new conversation. |
| Visuals, Files, Browser and Terminal views, pane overview, Room switching (11, 30, 31) | Link. | Link. | Link. | Not terminal features. Out of scope by design. |

### 6.1 The fallback: a link to the same pane

The web client already opens a Room at an agent pane from `/terminal?kernel=…&session=…&agent=…` (Cloud `apps/web/src/terminal-os/use-startup-target.ts`). Nothing in OSS builds that link today, and the kernel would need the web address it is paired with. With that, every card the terminal cannot answer becomes one line: what is waiting and a link that opens the pane with the card in it, on the web or on the phone. The link carries no card content.

### 6.2 Terminal-side overlays

Drawing over the provider's screen from the launcher (a reserved status row, a popup) needs a full terminal emulator between the provider and the real terminal, because the providers position the cursor and set scroll regions themselves. That is a second renderer to keep correct against three moving UIs. It is not recommended. Two things the launcher can write safely because they are outside the screen grid: the terminal title and a bell or desktop-notification sequence.

## 7. Recommended shape

### 7.1 One kernel path: the pane feed

The kernel adds one read model per agent, the pane feed, built from state it already owns:

- identity: Room, agent, sub-agent path, Project, placement;
- settings as the kernel records them: provider, account, model, effort, mode, permissions;
- state: idle, working, waiting for an answer, waiting for the person in the terminal;
- cards shown in this agent's pane, its own and its sub-agents', each with the asker's name, the choices, a flag "can be answered in a text terminal" and the link of section 6.1;
- stripes, usage and notices, with their wording.

Decisions:

1. **The kernel builds the feed and its wording; adapters only translate.** Runtime rule 1. Today the stripe and status wording is computed in the web client, so three adapters would each invent their own. (P3: the same words everywhere.)
2. **One `RuntimeInteraction` per prompt; every answer goes through `RespondToInteraction`.** An adapter never decides. First answer wins, and the adapter closes the provider's dialog when the kernel says the card is settled. (P1: answer anywhere, it disappears everywhere.)
3. **The kernel, not the adapter, decides which cards a text terminal may answer.** Passkey choices, pages, payments and file picks are link-only, because a text terminal cannot draw them. A secret or passphrase the person types is answered in the native form like any typed reply: guarding what a person types protects nothing the model would otherwise see (AGENTS.md). The Vault rule stays in one place: the kernel never puts a Vault value into a provider's form, prompt or output. (P2 and the secrets rule.)
4. **A card that blocks the agent's running turn opens its dialog at once. A card that arrives while the person is idle or typing only appears in the status line or as a notice, and opens on request.** A dialog that pops under the person's fingers can be answered by accident; in Claude Code Enter accepts a no-field form. (P1.)
5. **Dismissing a dialog in the terminal means "not now".** The card stays open for every client. Only an explicit choice answers it. (P1.)
6. **Provider-native dialogs, not Chariox-drawn ones.** A Claude user answers a Claude dialog, a Codex user a Codex overlay. No overlay renderer (section 6.2). (P3.)
7. **Room commands stay one catalog in the kernel.** The terminal gets a way to reach it, not a second list. Asking the agent is always available, because agents already run Room commands themselves. (P2.)
8. **The feed is a new serialized shape.** It needs the protocol version bump and a conformance drill, per the protocol-change rule.

### 7.2 Claude Code

Everything is in files and endpoints the kernel already writes or serves for a native Claude run: the settings file, the hook script, the status line command and the Chariox MCP endpoint. Nothing new runs in the launcher.

- Status line: line 1 identity and settings, line 2 state and cards waiting with a link, line 3 stripes. Refresh every 2 s. (P1, P3.)
- Provider permissions: Claude's dialog and the kernel card stay one prompt. Fix F3 with the close-out of section 6. (P1.)
- Kernel cards: MCP elicitation, mapped from the feed. Link-only cards use URL mode. (P1, P3.)
- Room commands: `/chariox …` through the UserPromptExpansion hook. Only text the person typed is expanded; kernel-injected prompt text never is. (P3.)
- Model and mode changes made in the terminal are reported to the kernel. (P1: one truth in every client.)

### 7.3 Codex

The adapter is the existing proxy. It reads the feed from the kernel and sends synthetic server requests and notifications.

- Permissions: extend today's projection to every permission card of the pane. (P1.)
- Kernel cards: form-mode elicitation requests, so the card-to-form mapping is the one Claude uses; `serverRequest/resolved` on settle. (P3.)
- Information: thread name for identity, one `warning` line per change, terminal title from the launcher. No persistent stripes. (P1: say less rather than draw something fragile.)
- Room commands: through the agent; `!chariox …` where no model turn is wanted. (P2.)
- The model and effort the terminal sends with a prompt are applied to the kernel run, so Codex's own footer is true. (P1.)

### 7.4 OpenCode

The adapter is the existing proxy plus one official TUI plugin that the launcher configures.

- Permissions: as today.
- Kernel cards: synthetic question events; replies and rejects are answered by the proxy and sent to the kernel. (P3.)
- Information and Room commands: the TUI plugin fills the footer and sidebar slots from the feed and registers `/chariox` as a real slash command. Toasts for notices. (P1, P3.)

### 7.5 What stays out

No overlay renderer, no screen scraping for new features, no provider SDK, no Claude Channels, no second card authority, no work on Chariox's own terminal UI.

## 8. Milestones

Lane-weeks are one lane working for one week. M2, M3 and M4 can run in parallel after M1.

| Milestone | Content | Lane-weeks |
| --- | --- | --- |
| M0 Repairs | F1, F2, F3, F8, F9 on today's path, each with a failing test first. F4, F6 and F7 are closed by later milestones. F10 needs a diagnosis first. | 1 |
| M1 Pane feed | Kernel read model, wording, text-terminal flag, link, protocol bump, conformance drill. | 1.5 |
| M2 Claude Code | Status line, cards by elicitation, permission close-out, `/chariox`, model and mode reports. | 2 |
| M3 Codex | All permission cards, cards by elicitation request, resolved, warnings, thread name, model applied. | 1.5 |
| M4 OpenCode | Question cards, toasts, TUI plugin footer, sidebar and command. | 1.5 |
| M5 Launch, resume and exit | Project, account, environment and "new Room" arguments; resume an existing agent (`--agent`, kernel-side resume of its provider session); terminal title and notification from the launcher; exit hint; drills on a slice and a remote machine. | 1.5 |
| | Total | 9 |

## 9. Validation cells

To be accepted by the owner before implementation starts. Each runs on the real kernel, the real provider terminal UI and a web pane on the same Room.

| # | Cell | Passes when |
| --- | --- | --- |
| V1 | Agent asks a question (kernel choice card), person at the terminal | The provider's own dialog shows the question and choices; one choice there continues the turn; the web pane shows the same card closing. All three providers. |
| V2 | Same card, answered on the web pane | The terminal dialog closes within 2 s and the turn continues. All three. |
| V3 | Person dismisses the dialog | The card stays open on the web pane and is listed in the terminal's status line or notice; nothing is answered. |
| V4 | Provider permission prompt, answered Yes, No and from the web pane | Every answer settles the one kernel card within 2 s; after "No" the next prompt from the web pane reaches the terminal (F3). |
| V5 | Vault unlock card and a typed-secret card | The person answers both in the provider's own form (a text field) or through the link; either answer settles the one kernel card; the kernel never puts a Vault value into the terminal, a provider form, history or logs. |
| V6 | Sub-agent raises a card | The parent's terminal shows it with the sub-agent's name. |
| V7 | Idle card (a wake fires a question while the person types) | No dialog opens over the typing; the status line or notice shows it; it opens on request. |
| V8 | Identity and settings | Claude and OpenCode show Room, agent, model, effort, mode and permissions as the kernel records them; a switch made in the terminal shows on the web pane within 2 s; Codex's own footer matches the kernel. |
| V9 | Room command | `/chariox goal …` (Claude, OpenCode) sets the goal with no model turn and the web pane shows the goal stripe; on Codex asking the agent does the same. |
| V10 | Launch | `chariox claude --project … --account … --slice …` opens the same Room setup the web client would; exit prints the Chariox command that resumes the same agent, and running it shows the earlier turns in the provider's terminal. |
| V11 | Codex: a server request with no running turn | The overlay opens while idle. If it does not, Codex cards outside a turn fall back to a warning line and the link. |
| V12 | Claude Code over the kernel's own MCP endpoint | An unsolicited elicitation from the kernel's HTTP MCP endpoint opens the dialog (proven with a scratch HTTP server, not yet with the kernel's). |

## 10. Open questions

Each has a recommended default.

| # | Question | Recommended default |
| --- | --- | --- |
| Q1 | If Chariox's own terminal UI is removed, where do the launchers live? | Keep the `chariox` binary as the launcher and command line; split `apps/cli/src/native-tui/` from the Ink UI so the UI can go without touching it. (P1) |
| Q2 | May the Claude path rely on the undocumented race between the PermissionRequest hook and Claude's dialog? | Yes, with a pinned-version drill that fails when the behaviour changes. The alternative, hiding Claude's dialog, does not exist. (P3) |
| Q3 | Claude's "always allow" and "switch to auto mode" options change rules outside the kernel. Allow them? | Allow them as the person's own choice in their own terminal, and report the resulting mode to the kernel so every client shows it. (P1, P3) |
| Q4 | Ordinary permission cards time out after 300 s with deny; the rule for questions is no deadline. Which one for a person sitting at the terminal? | No deadline while a terminal is attached to the pane; keep the timeout only when no client is attached. (P1) |
| Q5 | May Codex cards use `item/tool/requestUserInput`, which Codex marks experimental? | No; use elicitation requests, which follow the MCP shape. Revisit if V11 fails. (P3) |
| Q6 | Should free OpenCode models run with no OpenCode account, as plain `opencode` does (F2)? | Yes. Refuse only models that need an account, and say which account to add. (P1) |
| Q7 | Which web address does the kernel put in links, and what does a kernel with no Cloud pairing show? | The address of the Cloud it is paired with. With no pairing, the line names the Room and agent with no link. (P1) |
| Q8 | One `/chariox <command>` or the 39 Room commands as separate slash commands? | One command. Separate commands would collide with the providers' own (`/model`, `/status`, `/permissions`). (P3) |
| Q9 | Should a native launch default to `required` or `yolo`? Today Claude differs from Codex and OpenCode. | The Room's default, as in the web client. (P3) |

### 10.1 Answers from the owner (2026-11-18)

| # | Answer |
| --- | --- |
| Q1 | Yes: keep the `chariox` binary as the launcher and command line. |
| Q2 | Yes: rely on the hook-and-dialog race, with a pinned-version drill. |
| Q4 | Yes: no deadline while a terminal is attached to the pane. |
| Q6 | Yes, limited to the models that run without an account in plain `opencode`. |
| Q8 | Yes: one `/chariox <command>`. |
| Q3 | Yes: Claude's "always allow" and "switch to auto mode" are the person's own choice; the resulting mode is reported to the kernel so every client shows it. |
| Q5 | Yes: Codex cards use elicitation requests, not the experimental `item/tool/requestUserInput`; revisit if V11 fails. |
| Q7 | Yes: links use the address of the Cloud the kernel is paired with; with no pairing the line names the Room and agent with no link. |
| Q9 | Yes: a native launch uses the Room's default permission mode, as the web client does. |

The owner also accepted the 12 validation cells of section 9 as written and the three repairs of section 1 (F1, F2, F3). Implementation starts with M0 and M1 (section 8). The adapters (M2, M3, M4) and M5 start once M1's protocol shape has a clean review.

### 10.2 Added by the owner

| # | Item | Passes when |
| --- | --- | --- |
| NT-LIMITS | A limitations section for users, in the docs and in `chariox <provider> --help` and the exit hint: what a native terminal cannot do and where it goes instead (a link to the same pane on the web or phone). | It lists page-region and browser hand-off cards, passkeys, payments, file picks, App confirmations, the Visuals, Files, Browser and Terminal views, Room switching and the pane overview (one terminal is one agent), capture region and the picture editor, dictation, drafts, archive, delete and carry, drag-and-drop images (partial), Codex's lack of persistent stripes (a warning line and the link instead), and that nothing is drawn over a provider's screen. |
| NT-NOTES | Leaving a note on a selection in a native terminal. The audit did not cover it: the selection belongs to the terminal and the provider. | Provider-native hooks, `/chariox note <text>` that refers to the last reply or turn, and the link to the pane are evaluated, and the result is either a line in NT-LIMITS or a validation cell. |

## 11. Known parked items this touches

From the lane ledger; none is changed by this audit.

- Claude stream fix #1425 (trailing `*` duplicated) is not in `main`.
- The live real-provider drill of the native prompt and Stop path had not been run; sections 3.3 to 3.5 are its first run.
- The ask-mode watched-process card in a native terminal is fixed on a branch with no open pull request.
- Tool row about 2.8 s late; live merge key restarts at 1 after a kernel restart; code-fence live tail issues.
- Hidden-context chunks are cut at 6,000 bytes mid-word.
- Decision pending: 300 s timeout and deny for ordinary permission cards against the no-deadline rule for questions (Q4).
- Blocked: slicecrash-live.
- Acceptance not run or failing: path1/MP-10-CLIENT03; path1/MP-08-RUNTIME (Codex and OpenCode adapters, terminal in a slice); transport/LD-06; path1/MP-10-FIRST-MANAGED-CLAUDE-DPR1 and DPR2 (the folder-trust card blocks the first prompt; provider accounts missing).

## 12. Sources

- Chariox OSS at `d687f3b911`: `apps/cli/src/native-tui/`, `apps/kernel/src/provider/claude/native_tui.rs`, `apps/kernel/src/provider/claude/native_tui_hook.mjs`, `apps/kernel/src/app/provider_output_claude_native.rs`, `apps/kernel/src/runtime/terminal_command_catalog.rs`, `docs/PROTOCOL.md` section 3.3.2.
- Cloud at `d8b8df0d49`: `apps/web/src/terminal-os/` (files listed in section 4).
- Claude Code 2.1.296: installed binary; documentation pages for hooks, status line, plugins, MCP and channels.
- Codex CLI 0.159.3: installed binary and its generated app-server schema; `openai/codex` at `rust-v0.159.3`, `codex-rs/tui/src/` and the app-server README; configuration documentation.
- OpenCode 1.19.0: installed binary and its OpenAPI document; `anomalyco/opencode` at `v1.19.0`, `packages/plugin/src/tui.ts`; server and plugin documentation.
- Evidence: `/root/.codex/evidence/orch12/nativetui-audit/` with `claude-live/`, `codex-live/`, `opencode-live/` (through Chariox) and `claude-exp/`, `opencode-exp/` (scratch servers, with their sources and logs).
