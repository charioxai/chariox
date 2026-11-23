# Notification center plan

Status: accepted for building. The owner answered the six open questions on 2026-11-17 (section 15). No product code exists for this plan yet. The clickable mock, its screenshots and its source are kept outside the repository under `/root/.codex/evidence/notifcenter/`.

Scope decided by the owner on 2026-11-17: one service with a sidebar entry "Notification center" and three tabs, Inbox, Subscriptions and Sources; one model and one delivery path; cards stay kernel-owned; Cloud never stores content. This document says how that is built on what the kernel already has, what is new, how it is validated and in which order.

Every decision this plan proposes names the principle behind it:

- **P1, simplest for the person:** fewest steps, no settings, no error text, no connection details.
- **P2, agents do it themselves:** a person step exists only for secrets, money, risky irreversible actions or another party's consent.
- **P3, familiar patterns:** every visible element follows the Web terminal and its prototype.

## 1. What the person gets

**Inbox.** Every card addressed to the person, from every Room on every machine they own, in one list. "Needs you" holds what blocks an agent: permissions, questions, sign-in pages and codes, App confirmations, payments. "For your information" holds what agents and sources wanted the person to know. The person answers a card right there. Answering it anywhere removes it everywhere.

**Subscriptions.** One table of everything that listens to something: which agent, workflow or person, in which Room on which machine, to which source, with which of the source's options picked, who created it and why, when it last fired, how often, and whether it is healthy. Each row can be paused, tested, pointed at another subscriber or removed, and shows what every past event caused.

**Sources.** Everything that can fire: the person's own watchers, public sources such as GitHub, workflow exits and App events. A new source starts from one sentence ("Tell me when the X100VI drops below €1,500 at fotokoch.de"). An agent writes the watcher and shows what it reads now and what it would have fired in the last 24 hours. The person names it, picks who hears about it and turns it on. No JSON, no cron, no script is ever shown.

**Agents can notify the person.** "Send me a notification when you are done" and "let me know when you receive the notification to review PR #976" both work. The agent's pane shows one line, "notified you: PR #976 is ready for your review"; the card is in the Inbox.

## 2. Model

```
source ──fires──▶ occurrence ──route (subscription)──▶ subscriber
                                                         ├─ agent     the kernel puts the occurrence in the agent's inbox and wakes it with a one-line prompt
                                                         ├─ workflow  the kernel starts a run at an entry point
                                                         └─ person    the kernel adds a card to the Inbox
```

| Word | Meaning | In the kernel today |
| --- | --- | --- |
| Source | Something that fires occurrences. It has an owner, a home kernel and a stable id. | `NotificationSourceKind`: `workflow_exit`, `workflow_run`, `owner_action`, `app_event` |
| Occurrence | One firing: `{source_id, display_name, occurrence_id, output, ancestry, deadline_ms}`; `output` is the message, the artifacts and, new, the structured `data`: the values of the fields the source's options name, and nothing else (section 6.2) | The notification envelope, 64 KiB at most, kept 7 days (`NOTIFICATION_RETENTION_MS`). Its `output` (`WorkflowOutputPayload`) holds `message` and `artifacts` only. |
| Route | A subscription row: one source, one subscriber. | `app_automations` rows, receipts in `app_outbox` |
| Subscriber | An agent, a workflow entry point or the person. | Agent (agent trigger), workflow entry point, outside agent |
| Card | Something shown to the person. | `RuntimeInteraction`, scoped to one session |

There is one delivery path, the existing one: a source commits an occurrence, the kernel delivers it once per subscription, across kernels over the end-to-end encrypted peer channel, with the existing answers (`Accepted`, `Duplicate`, `Expired`, `LoopDropped`) and the existing loop drop by ancestry. A public source reaches a kernel as it does today: `GitHub -> generator -> event delivery -> App inbox route -> App handler -> automation`. A watcher is a new kind of source on that path, and the person is a new kind of subscriber on it. Nothing else delivers.

The kernel gives events no meaning. A source emits opaque events; the script that produces them and the agent that receives them own the meaning.

## 3. What exists and what is new

| Part | Exists (local protocol 1010) | New |
| --- | --- | --- |
| Sources | Workflow exits and runs, owner actions, App events (`ListNotificationSources`) | Kind `watcher`: a supervised script on a kernel. Every source can declare options (section 6.2). |
| Subscribers | Agent, workflow entry point, outside agent | The person |
| Occurrence data | None. An App event is published as its `payload["text"]` alone (`app_event.rs`); the rest of the payload stays with the App | `data`: the values of the declared fields the source's options name travel in the occurrence, so the kernel has something to match (section 6.2) |
| Subscription fields | Source and target only: "a subscription has no options" | The values picked from the options the source declares (section 6.2), `once`, `paused`; the trigger prompt becomes required for agent-created subscriptions and is shown as "Why it exists" |
| Coalescing | None: every occurrence is delivered | On the source, not the subscription: every event, latest per key, debounce; one rate limit |
| Delivery | Once per subscription, cross-kernel, loop drop, 7 days | Unchanged |
| Cards | `RuntimeInteraction` per session, answered with `RespondToInteraction`, shown to every client of the session | A kernel-wide list of the cards addressed to the owner; for-your-information cards; read, snooze, mute |
| Counts | `pending_interaction_count` per session in the per-Room summary, for the Room's dot | The sidebar badge, counted from the Inbox |
| Agent tools | `chariox.notification.{sources,subscribe,unsubscribe}`, `chariox.request_popup` | `chariox.notify_person`; `options` and `once` on subscribe, and the source's declared options in `sources`; `chariox.watcher.*` |
| Lists | `ListWorkflowNotifications {session_id}` per Room | One kernel-wide list with last fired, count, health and flags |
| Trace | `ReadAgentTriggerTranscript`, delivery receipts | One trace per subscription: occurrence, result, the turn or run it started |
| Test | `TestAppInboxRoute`, `InvokeAgentTrigger` | `TestNotificationSubscription`: a dry run that sends nothing |
| Supervision | The jobs supervisor, the process watch behind agent wakes, the folder watch in `machine_files/watch.rs` | The watcher runtime, built on those |
| Workflow responsible | Lands with #1377 (`WorkflowDefinition.responsible`, `WorkflowRun.attention`) | Shown as subscription rows; attention for a person becomes a card |
| Clients | The Web terminal shows cards as stripes in the agent's pane | The Notification center program |
| Cloud | Signs in, lists the owner's kernels, issues relay tokens | Nothing in v1. Later: an opaque phone ping |

## 4. Inbox

### 4.1 Cards

```
InboxCard {
  card_id, created_at_ms,
  origin: { kernel_id, session_id?, agent_id? | workflow_id? | source_id? },
  needs_you: bool,
  kind,                      // permission, sign_in, question, app, payment, notice, source, workflow
  title, body?, link?, priority,
  read, snoozed_until_ms?,
  interaction?: RuntimeInteraction
}
```

A **needs-you** card is a pending `RuntimeInteraction` and nothing more: the card carries the interaction as it is, so every kind that exists today (choice, permission, the Vault live page region, sudo, App install, App confirmation, payment, provider sign-in, handoff) appears in the Inbox without a second representation. The Inbox adds no state to it. *(P3: the card in the center is the card in the pane.)*

A **for-your-information** card has no interaction. It comes from:

- an agent calling `chariox.notify_person`;
- an occurrence of a source the person is subscribed to;
- the health of a source the person is responsible for ("Visa appointments can't read the page any more", with Fix);
- a workflow run that needs attention when the workflow's responsible is the person (#1377).

Such cards are kept for 7 days, the existing retention constant, and at most 1,024 per owner per kernel, oldest read first. *(P1: no retention setting.)*

### 4.2 A view over the owner's kernels

Each kernel keeps the cards of its own Rooms and answers two things to its owner's clients: the current list with a revision (`GetPersonInbox`), and its changes (`PersonInboxChanged`) on the kernel-level connection the Web terminal already keeps for the Room list. The client holds one list per connected kernel and shows them merged. No kernel knows another kernel's cards, and Cloud knows none: it only tells the client which kernels the person owns, as it does today.

The sidebar badge is the number of needs-you cards that are not snoozed, plus the unread notices an agent sent because the person asked to be told (`origin: agent`), counted by the client from those lists. A notice is also a dismissible card in its agent's pane; dismissing it marks the one kernel item read, so the pane card and the center entry are one thing. The kernel's own notices (`origin: kernel`) are not requested notifications: one that carries an action (sign in to an account) is a person action and counts as needs-you; one without (a workflow that could not start) is status, in the center only. *(P1: the number means "things waiting for me"; no other informative event raises it.)*

### 4.3 Acting from the center

Answering a needs-you card sends the existing `RespondToInteraction` to the card's kernel. The interaction resolves once in that kernel; the kernel tells every client of the session and every Inbox listener, so the stripe leaves the agent's pane, the center and any other device together, and the agent continues. Answering in the pane does the same to the center. There is one record and one answer, so "closes everywhere" needs no synchronisation. *(Runtime rule 5: one kernel-owned interaction.)*

A live page card opens the same popup as in the Room: the cropped region of the kernel browser, typed into from the center. The Vault-filled field stays masked, and no Vault value reaches the model.

### 4.4 Read, snooze, mute

- **Read** applies to for-your-information cards. "Mark all read" exists once, on that section.
- **Snooze** (for 1 hour, until tomorrow) hides a card until then. A snoozed needs-you card still blocks its agent and stays in the Room.
- **Mute** applies to an origin: one agent, one workflow or one source. Its for-your-information cards arrive read and never push. Needs-you cards cannot be muted, because an agent is waiting. *(P1: a muted origin can never silently stall work.)*

All three are stored in the card's kernel (`SetInboxCardState`, `SetInboxMute`), so they follow the person to every client.

### 4.5 Grouping, opening, machines

The list is flat by default, newest first, with the origin on each row ("Reviewer · PR review · MacBook"). One button groups it by machine and Room. "Open in Room" opens the Room beside the center with the agent's pane focused on the turn that raised the card.

Machines follow the standing rule: no offline machine appears. When a machine's connection drops, its cards stay under "Reconnecting" for up to 10 minutes with their actions disabled, then leave the list. A stopped managed machine shows one row, "Stopped", with Start; its cards return when it runs.

### 4.6 Guests

v1 shows the kernels the person owns. A card in someone else's Room stays in that Room and never appears in the person's center, from v1 on, until the owner decides otherwise (decision 4, section 15). *(P1.)*

## 5. Agents notify the person

### 5.1 The tool

```
chariox.notify_person { title, body?, link?, priority? }   ->  { card_id }
```

- `title` is one line; `body` is optional detail; `link` is a URL or a Chariox target (a Room, a file, an artifact); `priority` is `normal` or `high`.
- The card is for-your-information. The agent does not wait and is not woken by it. When the agent needs an answer it uses `chariox.request_popup`, as today.
- The pane's transcript shows one line, "notified you: <title>". The card lives in the Inbox.
- `high` sorts first and is the only priority that will reach the phone later (section 12). *(P1: two values, no settings.)*
- The tool is not tied to the end of a turn. A turn ending raises nothing by itself; the Room's activity dot already says that. *(P1: an Inbox that fills with "done" is one nobody reads.)*
- An origin that sends more than the source rate limit (section 7.6) has its further cards folded into one ("Reviewer sent 14 notifications") and is flagged Noisy.

### 5.2 A request that must outlive the agent's memory

"Let me know when you receive the notification to review PR #976" must still work after the agent's context is compacted, the provider restarts or the kernel restarts. The kernel keeps the request, not the agent:

```
chariox.notification.subscribe { source, prompt, options?, once? }
```

- `prompt` exists today: it is the text the kernel queues together with the quoted occurrence. It becomes required for subscriptions an agent creates, and the agent writes into it what it was asked and what to do: "Miguel asked: let me know when you receive the notification to review PR #976. Notify him with a link to the PR." That is the stored intent. The Subscriptions tab shows it as "Why it exists".
- `options` is new (section 6.2): the values the agent picks from the options the source declares; here the GitHub source's `pull_request = 976`.
- `once` is new: the subscription removes itself after its first delivery.

When GitHub reports the review request, the kernel wakes the agent with that prompt and the occurrence. The agent, even with an empty memory, reads what it promised and calls `chariox.notify_person`. The trace of the subscription (section 6.4) shows the three steps: the event, the turn it woke, "notified you".

When there is nothing for the agent to judge, it subscribes the person instead of itself (`subscriber: "person"`), and the occurrence becomes a card without waking anyone. *(P2: the agent chooses; the person only said one sentence.)*

"Send me a notification when you are done" needs no subscription: the agent calls `chariox.notify_person` when its work is finished.

## 6. Subscriptions

### 6.1 The table

One row per subscription, from every kernel the person owns:

| Column | Content |
| --- | --- |
| Subscriber | The agent, workflow or "You", with Room and machine. A workflow's responsible (#1377) is a row of its own, marked "Responsible for release-pipeline". |
| Source | Name and kind, with the machine it runs on |
| Only when | The picked options as a sentence ("Only PR #976"), or "Every event" |
| Created by | The person or the agent that created it |
| Last fired, Fired | Time of the last delivery and the count |
| Health | OK, Paused, or a flag |

Row actions: **Pause / Resume**, **Test**, **Change subscriber**, **Remove**. Selecting a row opens its detail: why it exists, the picked options and the recent events.

Each kernel answers `ListNotificationSubscriptions` for the subscriptions whose subscriber lives on it; the client merges, as for the Inbox. The row data is the existing subscription (`source_available`, `source_kernel_id`, `created_by_user_id`, `target_kernel_id`) plus the trigger prompt, the picked options, the pause state and the counters. The Room's own list of workflow notifications becomes this table filtered to the Room, so there is one list and one implementation.

### 6.1a Delivery to an agent: the inbox and the mode

*(Owner, 2026-11-18; supersedes the 2026-11-03 rule that a notification always queues.)*

**One path for every source.** A GitHub or other App event, a workflow exit and an agent event reach an agent the same way. The kernel writes the whole occurrence into the agent's inbox (kind `notification`, state `accepted`, the occurrence's payload up to 256 KiB, once per subscription and occurrence id) and then prompts the agent with the subscription's own text followed by one line: `Notification: <title> (from <source>). Read it with chariox.events.inbox after <sequence-1>, then chariox.events.ack it.` The title is the first line of the occurrence (an App event: the event's title). The occurrence's text, payload and output never enter the prompt: the `<chariox-trigger-input kind="notification">` block is deleted, and there is no second delivery path. The agent reads the payload with `chariox.events.inbox` and acknowledges it. The prompt is the wake; the inbox is the data.

**Mode.** Every agent subscription has a `mode`, `queue` (default) or `steer`; it is the trigger's `busy_policy`. In `queue` the one-line prompt waits in the agent's prompt queue behind the running turn, and its row keeps the **Steer** button; the person or the agent can send it now or cancel it. In `steer` the prompt is steered into the running turn at once, as a queued-prompt steer is, and a refused steer leaves it queued. An idle agent starts a turn in both modes. The agent sets the mode when it subscribes (`chariox.notification.subscribe {mode?}`); the person changes it in the Subscriptions table (`UpdateNotificationSubscription {mode}`), and **Change subscriber** keeps it.

**Protocol (local 1012).** `NotificationSubscriptionRow.mode` (`queue|steer`, absent for a workflow or person subscriber), `NotificationSubscriptionChange::Mode {mode}`, `CreateAgentTrigger.busy_policy` and `UpdateAgentTrigger.busy_policy` accepted for notification triggers, `ReplacedListener.busy_policy`. The peer protocol is unchanged: the mode is the subscriber's own, and no peer carries it.

### 6.2 Source-declared options

The protocol says today that a subscription has no options. This plan adds exactly one kind that changes delivery, and it belongs to the source, not to the subscription: each source declares the few options a subscriber may pick, because without them "only PR #976" wakes an agent, and costs a turn, for every review request in the repository. There is no filter language: no operators, no expressions, no paths, nothing free-form that an agent or a person must write. *(P1: pick, never write. P2: an agent picks from a list and cannot get the syntax wrong. P3: a source offers choices the way any service's notification settings do.)*

- **What a source declares.** A list of at most 4 options, each `{name, label, type, field}`: `name` is a short identifier ("pull_request"), `label` is what the picker shows ("Pull request"), `type` is number, text or true/false, and `field` is one declared field of the source's events that the option compares. A source that declares none has none, and every subscriber gets every event. Where the declarations come from:
  - An **App or GitHub/AEGS generator** declares them with its event types. Its event type already declares a `payload_schema` (`docs/AEGS_SDK.md`, `docs/EVENT_TRIGGER_PROTOCOL.md`); an option is one field of that schema, such as `repository` and `pull_request` for a review request.
  - A **watcher** declares them in its manifest, each tied to one field of the `payload` its `event` lines carry (section 7.3).
  - A **workflow exit** declares none in this plan: its occurrence carries a message and artifacts, and a subscriber that wants less asks for a narrower workflow.
- **What the picker offers.** The subscribe picker, for a person ("Subscribe me" on a source) and the agent tool alike, offers exactly the declared options and nothing else; a subscription picks a value for none, some or all of them. `chariox.notification.sources` lists each source's options with their types, so an agent never guesses.
- **Exact match.** For each option the subscription picked, the event's declared field must equal the picked value, of the same type: the number 976 matches 976 and not 9760, and not the text "976". Every picked option must match. An event that lacks a declared field does not match an option on it. There is no `!=`, no `<`, no `contains`.
- **Refused at subscribe time.** The subscriber's kernel checks a subscription against the source's declaration (local, or in the peer source list for a source on another kernel) when it is created. An option the source does not declare, a value of another type than the declared one, or a repeated option is refused with the list of options the source does declare, and no row exists. An agent that gets the refusal corrects itself from that list. *(Fail clearly: nothing is stored half-understood.)*
- **A narrow source needs no option.** A watcher created from a prompt is made narrow through its own parameters: "tell me when PR #976 is reviewed" becomes a watcher for that PR. An option exists for the case where several subscribers of one source want different slices of it, not to compensate for a source that is too wide.
- **Where `data` comes from.** Today an occurrence carries no structured data: `WorkflowOutputPayload` (`session/workflow_outputs.rs`) holds `message` and `artifacts`, and the App-event adapter (`workflow_notifications/app_event.rs`) publishes `payload["text"]` and drops every other field, so the GitHub payload that holds the PR number never reaches a subscription. `WorkflowOutputPayload` gains an optional `data`: a flat object holding the values of the declared option fields of that event, and nothing else. (1) The App-event adapter copies those fields from the event's payload, by the source's declaration; `text` stays the `message`. (2) A watcher's declared fields are copied from the `payload` of its `event` line. (3) A workflow exit carries no `data`. (4) `data` is part of the occurrence the kernel already stores in `app_outbox` and the occurrence tables, and of the envelope `DeliverWorkflowNotification` carries across kernels, so the peer channel needs no second message. (5) `data` holds only the declared fields, so it stays small inside the 64 KiB output limit. (6) The subscribed agent receives `data` quoted with the message, as untrusted input (section 11). The rest of the event's payload never leaves the source.
- **Where it is applied.** The subscriber's kernel matches the picked values against the occurrence's `data` when it accepts the occurrence, before anything is queued. The source's kernel is answered `Accepted` as today; the receipt records "Filtered out", and a filtered occurrence does not use up a `once` subscription. A subscription with no picked option matches every occurrence.
- **A source that changes.** If a source later stops declaring an option, or declares it with another type, a subscription that picked it is paused, flagged "Options changed" (section 6.5) and never widened to every event. *(P1: a narrow subscription never silently becomes a flood.)*
- A filtered occurrence is recorded in the trace as "Filtered out", worded for the picked options: "Not PR #976". It is delivered to nobody.

Coalescing is not a subscription option. It belongs to the source (section 7.6), so every subscriber of a source sees the same events. *(P1: one place decides how often something fires.)*

### 6.3 Test

**Test** is a dry run and only a dry run: the kernel takes the source's latest occurrence (or a sample the source declares), applies the picked options and reports what would happen ("Would wake Reviewer. Nothing was sent."). Nothing is queued and no counter moves. *(P1: testing can never wake an agent or start a run by accident.)*

### 6.4 Trace

Every occurrence leaves one line per subscription, kept as long as occurrences are (7 days):

| Result | Shown as |
| --- | --- |
| Woke an agent | "Woke Reviewer, who notified you", with **Open turn** |
| Started a workflow run | "Started run 41 of release-pipeline: completed", with **Open run** |
| Became a card | "Shown in your Inbox" |
| Filtered out | "Filtered out: Not PR #976" |
| Not delivered | "Paused", "Expired after 7 days", "Loop stopped" |

For an agent, the delivery receipt records the turn the queued prompt started (`turn_ref`), and when the turn ends, one line of what it did: its person-facing effects first ("notified you", "asked you"), otherwise the first line of its answer. **Open turn** opens the Room at that turn through the existing history read. `ReadNotificationTrace {subscription_id}` returns these lines.

### 6.5 Flags

| Flag | Raised when | What the person can do |
| --- | --- | --- |
| Source is gone / Subscriber is gone (orphan) | `source_available` is false, or the agent or workflow no longer exists | Change subscriber, Remove |
| Loop stopped | A delivery was answered `LoopDropped` | Open the trace; Remove |
| Noisy | The source or origin hit the rate limit | Fix (an agent adjusts the watcher), Pause |
| Source stopped | The watcher failed or its machine is stopped | Fix, or Start the machine |
| Options changed | The source no longer declares an option the subscription picked, or declares it with another type. Delivery pauses. | Change subscriber, Remove, then subscribe again from the source's current options |

On one kernel, the kernel refuses a cycle when the subscription is created (`workflow notification cycle`): a workflow subscribed to its own exit, and the second subscription of A to B to A, are both refused, so there is no row and no flag. Only a cycle across kernels can form, because each kernel sees only its own subscriptions; there the occurrence's ancestry stops it at delivery, answers `LoopDropped` and raises Loop stopped.

A flag also raises one for-your-information card to whoever is responsible for the subscription: its creator, or the workflow's responsible.

## 7. Sources and watchers

### 7.1 The table

One row per source: name with its sentence ("Watching the visa appointment page"), kind, the machine it runs on ("Chariox" for a public source), how it is checked ("Every 15 min · 2m ago", or "Live"), last fired, subscribers, health. Row actions for a watcher: **Check now**, **Pause**, **Move to <machine>**, **Change**, **Remove**. For a public source, a workflow or an App event: **Subscribe me** (with the source's declared options, when it has any), **Open**.

### 7.2 Creation

1. **Prompt and examples.** "What do you want to watch?", one large prompt and a grid of examples (a price, a folder, a feed, a log, an inbox, a calendar, a deploy, a machine's disk). Choosing an example fills the prompt.
2. **The agent builds it.** A utility agent that belongs to the center runs on the machine where the watcher will live, in a Room of its own, so its trial runs don't clutter the person's working agents. The left side shows the conversation in plain steps ("Opening fotokoch.de…", "Found the price: €1,649.00, in stock"). The agent asks only what it cannot find out. Secrets go through the Vault as everywhere else. What the person asks for makes the watcher narrow through its own parameters ("only PR #976" is a watcher for PR #976); the agent declares an option (section 6.2) only where several subscribers would want different slices of the same source. *(P2.)*
3. **Preview.** The right side shows one sentence ("Watching the X100VI page at fotokoch.de · every 30 min · on MacBook"), the current reading and the dry run (section 7.4).
4. **Placement**, only when it matters (section 7.7).
5. **Name and subscribers.** A suggested name, and "Who should hear about it?": the person, and the agents and workflows of their Rooms, as a checklist. When the source declares options, the same screen offers them for each subscriber (section 6.2).
6. **Turn on.** Until then nothing runs on a schedule and nothing is delivered. Discard removes everything.

"Change" on a source reopens the same conversation: "only on weekdays", "check less often". Every change shows a new dry run before it applies.

Any agent can do the same through tools, without the screen and without the person: `chariox.watcher.create`, `dry_run`, `activate`, `status`, `remove` (section 8.3). *(P2.)*

### 7.3 The watcher

A watcher is a package the kernel supervises: a directory in the kernel's state with a manifest and a script. The agent writes both; the person sees neither.

Manifest:

| Field | Meaning |
| --- | --- |
| `sentence` | The one line shown to the person |
| `mode` | `poll` (run every `every_s`, then exit), `stream` (stay running: tail, file watch, socket) or `push` (run once per request to an address the kernel serves on its machine, as the agent HTTP trigger does) |
| `command` | The argv to run |
| `events` | The event names it can emit, each with whether it carries a key |
| `options` | Optional, at most 4: the options a subscriber may pick, each `{name, label, type, field}` tied to one field of the `payload` its events carry (section 6.2) |
| `coalesce`, `debounce_s` | Section 7.6 |
| `credentials` | Vault handles and the environment names they fill |
| `sees` | What it needs from its machine: folders, local commands, hosts. Decides where it can move (section 7.8). |
| `judge` | Optional: the question for "changed meaningfully" (section 7.5) |

Contract: the kernel writes one JSON line to the script's standard input and reads JSON lines from its standard output.

```
in    {"cursor": {...} | null, "now_ms": 0, "dry_run": false, "since_ms": null}

out   {"reading": "€1,649.00 · In stock"}
      {"event": "price_below", "key": "x100vi", "text": "X100VI is €1,479 at Foto Koch", "payload": {...}}
      {"candidate": {"key": "slots", "before": "...", "after": "..."}}
      {"history": false}
      {"cursor": {...}}
```

- `reading` is what the source currently sees, in words. It is shown in the table and the preview.
- `event` becomes an occurrence: `text` is its message, and the fields its `options` name are copied from `payload` into the occurrence's `data`. The rest of `payload` stays with the script. An `event` that lacks a declared field is still delivered; it matches no subscription that picked an option on that field.
- `cursor` is the script's memory (last seen id, last value, a hash). The kernel keeps it, 64 KiB at most. The script keeps no state of its own, which is what makes the watcher portable.
- The kernel commits a run's events and its cursor in one transaction, so a crash or a restart neither repeats nor loses an event. A `poll` run commits when it exits with 0; a `stream` commits at each `cursor` line.
- Credentials reach the script as environment values filled from the Vault by handle, the way `chariox.vault.use_env` fills them. They are never written to the package, the cursor, a reading or a log.
- The kernel supervises with the jobs supervisor: start and stop, a time limit per run, restart with backoff for `stream`, process limits, a bounded log, and survival across kernel restarts.

**Failure is an event.** A run that fails, a `stream` that keeps dying, a gap since the last good run, and a source slowed for noise each change the source's health and fire once to whoever is responsible for the watcher: a card for a person, a wake for an agent. The card is a sentence with a repair action, never an error text: "Visa appointments can't read the page any more. Fix it?" Fix reopens the conversation with the failure; the agent repairs the script and shows a dry run. *(P1.)*

### 7.4 Dry run

A dry run calls the script with `dry_run: true` and `since_ms` 24 hours back. The script reports what it would have fired since then; the kernel delivers nothing and returns the list to the preview.

- **Sources that keep history** (feeds, mailboxes, logs, folders, GitHub and GitLab, APIs with a `since`): the preview says "Would have fired 3 times in the last 24 hours" and lists them. This is exact, not an estimate.
- **Sources that keep none** (a web page, a machine's health, a process): the script answers `{"history": false}`, and the preview says so instead of inventing a history: "This page keeps no history, so I can only read it as it is now. Now: €1,649.00. It would not fire." The agent then proves the condition on a changed copy of what it read: "Tested with a copy of the page at €1,479: 'X100VI is €1,479 at Foto Koch'." *(P1: the preview never claims what it cannot know.)*

Once a watcher is on, its real history replaces the dry run in the same place.

### 7.5 "Changed meaningfully"

Cheap first, a model only past a threshold:

1. The script compares the new reading with the cursor: normalised text, a number, a hash. No difference, or a difference under the watcher's threshold (a moved timestamp, a rotated advert), emits nothing.
2. A difference the script can decide (a price under a limit, a new feed item, a matching log line) emits an `event` directly. Most watchers never go further.
3. Otherwise the script emits a `candidate` with short before and after excerpts. The kernel asks a small model one question, the watcher's `judge` ("Did the availability of appointments change?"), through the utility-call path (`utility/watcher-judge.md`), at most once per check. Yes becomes an event with the model's one line; no becomes nothing.

If the model cannot be reached, the candidate is delivered as "changed (not judged)". A missed appointment is worse than one extra card.

### 7.6 How often a source fires

Set by the agent in the manifest, once per source:

- **Every event**, or **latest per key**: an undelivered occurrence is replaced by a newer one with the same key.
- **Debounce:** wait for quiet before firing.
- **Rate limit:** one kernel constant per source. A source that exceeds it slows itself ("Noisy: slowed to 1 a minute"), keeps the latest per key, and tells its responsible, with Fix.

### 7.7 Placement

A watcher runs where it can see what it watches and, if it matters, where it keeps running.

- What it watches is on a machine (a folder, a log, a process, a LAN device): it runs there. Nothing is asked.
- What it watches is on the network (a page, a feed, an API, a mailbox): it runs on an always-on machine the person already has; otherwise on the machine in use.
- When that machine is not always on, the agent says so once and may suggest one with its price: "MacBook and Workstation sleep at night, so checks would pause then. An always-on machine costs €4.51 a month." with **Keep on MacBook** and **Rent for €4.51 a month**.

The agent never rents or places on a paid machine by itself. Renting is the person's click and goes through the existing rent confirmation. Keeping it where it is costs nothing and is always offered first. *(P2: money is a person step. P1: one question, two answers.)*

### 7.8 Moving a watcher

Watchers are portable: the package, the cursor and Vault handles are all there is. "Move to <machine>" lists the person's connected machines the watcher's `sees` allows. A folder watcher is not offered another machine. A machine the person adds later, such as their own server after installing Chariox, appears in that menu like any other.

A move is one durable record with a `move_id`, kept by both kernels until it ends. Running a source is owned by one kernel at a time; the owner is the kernel that holds the source's highest `source_generation`. That is a new field of the source record: the existing notification generation (`source_fence.rs`) orders a subscription's attach and detach requests and says nothing about which kernel executes a source. The commit point is step 4, one transaction on the new kernel.

Move, between the two kernels over the encrypted peer channel:

1. The old kernel records the move, pauses the watcher and fixes its cursor. It keeps its copy paused until the move ends.
2. It sends the `move_id`, the package, the cursor and the undelivered occurrences. Vault entries the watcher uses travel through the Vault's own transfer, as when an agent moves (`docs/MOVE_AGENT_TO_KERNEL.md`). Occurrences are identified by source id and occurrence id, which the kernel already ignores when it sees them twice, so a resend delivers nothing twice.
3. The new kernel records the move as prepared, installs the package and runs one check without delivering and without scheduling anything.
4. **Commit.** In one transaction the new kernel marks the move committed, registers the source under the same source id with `source_generation` raised by one and starts its schedule. Subscriptions on any kernel follow, as they follow a workflow source today. A watcher's declared options travel with it, unchanged.
5. The old kernel, told that the move committed, removes its copy.

What happens on a failure depends on which side of step 4 it falls:

- **Before the commit** (a failure in steps 1 to 3, or the new kernel answering that it refuses): the move is rolled back. The old kernel resumes from its own cursor, the new kernel drops what it installed, and a card says the move did not happen. Nothing fired on the new kernel, so nothing fires twice.
- **After the commit** nothing is rolled back; the move only goes forward. The new kernel owns the source. The old kernel never resumes its copy once it has learned the commit, and removing the copy (step 5) is retried until it succeeds, by `move_id`. A card says the move finished.
- **When the outcome is not known** (the reply to step 4 is lost, or the kernels cannot reach each other after step 3), the old kernel does not guess. It stays paused, resumes nothing, and asks the new kernel for the status of the `move_id`, repeatedly and with backoff, until it is answered `committed` or `aborted`. The answer is idempotent. Asking for an abort is the only way to resume the old copy: the new kernel, in one transaction, either answers `committed` or records `aborted` and refuses any later commit of that `move_id`. So exactly one of the two runs the watcher, whichever way the failure falls. While the new kernel cannot be reached the watcher is paused, not duplicated, and a card says so with one action, "Keep it here", which asks for the abort and applies when the new kernel answers. A source with history catches up from its cursor when it resumes; one without history (the dry run's second case) can miss what changed in that time, which the card says.
- **A restart of either kernel** at any of these points reads its record of the move and continues from it: the old kernel with the status question, the new kernel with the commit or the abort it already made.

Because the cursor moved with the watcher and only one kernel owns the source at any time, nothing fires twice, and nothing fires from an old cursor after the commit.

### 7.9 Catalog

The catalog is what the example grid and the utility agent's instructions cover. Every kind is the same runtime; the kinds differ only in the script the agent writes.

| Kind | Example | Mode | History for the dry run | Runs on |
| --- | --- | --- | --- | --- |
| Files and folders | "New PDF in Documents/Invoices" | stream | Yes (file times) | The machine that has the folder |
| Schedule | "Every weekday at 9" | poll | Yes | Any |
| Web page change | "The visa page shows a free slot" | poll; the kernel browser only when the page needs it | No | Any |
| Feed (RSS, Atom) | "Hacker News mentions Chariox" | poll | Yes | Any |
| Mailbox (IMAP) | "An email from the tax office" | poll or stream | Yes | Any, with a Vault credential |
| Webhook | "My door sensor calls this address" | push, for senders on the machine or its network; a sender on the internet needs a public source | No | Any |
| GitHub, GitLab | "Review requested on PR #976" | Public source (hosted), or poll with a token | Yes | Chariox, or any |
| Slack | "Someone mentions me in #ops" | Public source, or poll | Yes | Chariox, or any |
| Calendar | "30 minutes before any meeting" | poll | Yes | Any, with a Vault credential |
| API poll with a condition | "Stripe balance under €500" | poll | When the API has it | Any |
| Log pattern | "New error line in relay.log" | stream | Yes | The machine that has the log |
| Process exit | "The backup job ended" | stream | No | The machine that runs it |
| Machine health | "Disk above 85%" | poll | No | That machine |
| App events | "The Invoices App received a payment" | Existing source | Yes | The App's kernel |
| Workflow exits and runs | "release-pipeline finished" | Existing source | Yes | The workflow's kernel |

**Coverage checklist.** Huginn's agent list is the checklist for "can a person ask for this", not something to import. Its agents fall into three groups; the names are to be re-checked against its repository when the catalog is written.

| Huginn group | Its agents (examples) | Here |
| --- | --- | --- |
| Detect | Website, RSS, IMAP Folder, Webhook, Scheduler, Local File, FTP Site, S3, Dropbox Watch, Http Status, Twitter Stream and Search, Github Notifications, Jira, Weather, User Location, MQTT, Shell Command, Aftership | A watcher kind above. Location and MQTT are `push` and `stream`. |
| Shape | Trigger, Change Detector, De-duplication, Peak Detector, Gap Detector, Attribute Difference, Delay, Digest, Event Formatting, JSON Parse, Csv | Inside the script (condition, cursor), the source's coalescing, or the subscribed agent. No chains or transform steps. |
| Act | Email, Slack, Pushover, Telegram, Twilio, Post, Data Output, Commander, Human Task | Not sources. The subscribed agent acts with its own tools; reaching the person is the Inbox. |

## 8. Kernel and protocol surfaces

Design level: names follow the existing requests, and the exact shapes are set in the implementing change together with the protocol version bump and snapshot update.

### 8.1 Local API

| Request | State | Purpose |
| --- | --- | --- |
| `RespondToInteraction` | Exists | Answer a needs-you card |
| `ListNotificationSources` | Exists, extended | Adds watchers with sentence, reading, check time, health, home kernel; every source lists its declared options |
| `CreateAgentTrigger`, `AttachWorkflowNotification` | Exist, extended | `options` (values picked from the source's declared options, refused at creation when undeclared or mistyped), `once`; the prompt required for agent-created subscriptions |
| `GetPersonInbox`, event `PersonInboxChanged` | New | The owner's cards on this kernel, and their changes |
| `SetInboxCardState {card_ids, read?, snoozed_until_ms?}` | New | Read and snooze |
| `SetInboxMute {origin, muted}` | New | Mute one agent, workflow or source |
| `SubscribePerson {source, options?, once?}` | New | The person as subscriber |
| `ListNotificationSubscriptions` | New | Kernel-wide rows with prompt, picked options, counters, health, flags. `ListWorkflowNotifications {session_id}` is deleted in the same change. |
| `UpdateNotificationSubscription {paused?, subscriber?, mode?}` | New | Pause, resume, change subscriber, change queue or steer (section 6.1a). The row and its history stay. |
| `RemoveNotificationSubscription` | New | One removal for every subscriber kind |
| `TestNotificationSubscription` | New | Dry run, sends nothing |
| `ReadNotificationTrace {subscription_id}` | New | Section 6.4 |
| `CreateWatcher`, `DryRunWatcher`, `ActivateWatcher`, `UpdateWatcher`, `PauseWatcher`, `CheckWatcherNow`, `RemoveWatcher`, `MoveWatcher {target_kernel_id}` | New | Section 7 |

### 8.2 Peer protocol

Unchanged in kind: `SubscribeWorkflowNotifications` and `DeliverWorkflowNotification` carry watcher occurrences as they carry workflow ones. New: watcher sources in the peer source list (#1367 already lets the owner's agents follow the owner's sources on other kernels), and the move in section 7.8: the move record (`move_id`), the commit, the idempotent status and abort question, and `source_generation` in the peer source list, where the highest generation names the kernel that runs the source. The peer source list also carries each source's declared options, so the subscriber's kernel can check a subscription against them when it is created. `RELAY_PEER_PROTOCOL_VERSION` is raised with it. The occurrence's new `data` (section 6.2: the declared option fields only) is a field of the envelope `DeliverWorkflowNotification` already carries, and with the declared options changes serialized shapes clients and peers read: the implementing change raises `LOCAL_DAEMON_PROTOCOL_VERSION` and `RELAY_PEER_PROTOCOL_VERSION`, updates the snapshot and adds the drill, as AGENTS.md requires.

### 8.3 Agent tools

| Tool | State |
| --- | --- |
| `chariox.notify_person {title, body?, link?, priority?}` | New |
| `chariox.notification.sources` | Exists; lists watchers too, and each source's declared options with their types |
| `chariox.notification.subscribe {source, prompt, options?, once?, subscriber?, mode?}` | Exists; `prompt` required, the rest new; `mode` is `queue` (default) or `steer` (section 6.1a); an undeclared option or a value of the wrong type is refused with the source's options. `subscriber` is the calling agent or `"person"`. |
| `chariox.notification.unsubscribe {trigger_id}` | Exists |
| `chariox.watcher.create`, `.dry_run`, `.activate`, `.status`, `.remove` | New |
| `chariox.request_popup` | Exists; its cards now also appear in the Inbox |

### 8.4 Storage

Watcher packages, cursors and health live in the kernel's state beside the existing automations. Subscriptions stay in `app_automations` with the new columns; receipts stay in `app_outbox` with the result and `turn_ref`; an occurrence's `data` is stored with its output. For-your-information cards and the read, snooze and mute state are a new table in the same store. The limits stay: 1,024 sources and 1,024 subscriptions per owner, 32 subscribers per source, 64 KiB per payload.

## 9. Client views

- **Web terminal:** a program in the sidebar, in the place of today's "Notifications" entry, after Vault and Extensions. It uses the Machines window's parts: tabs with one action on the right, the inventory table with a row menu, an inspector on the right for a selected row. Cards are the work stripes of the agent pane, with the origin added. The popup is the Room's interaction popup. The creation screen is the prompt composer and a grid. *(P3: no new component.)* The mock shows every state.
- **iOS:** later, the Inbox first (section 12).
- **Terminal client:** none.

## 10. Traceability

The chain is always: what fired (the source and its reading), what was routed (the subscription and its picked options), what it caused (the turn, the run or the card), and what the person did (answered, read). The Subscriptions trace walks it forward from an event. A card walks it backward: its origin names the agent, the Room and, for a source's card, the subscription. "Why did I get this?" and "why did this agent wake up?" are each answered in one click from where the question arises.

## 11. Privacy and security

- **Cloud holds no content.** Titles, bodies, readings, events, scripts, counts and read state exist only in kernels and in the client's memory. The client reads each kernel through the relay, which carries only encrypted traffic. Cloud keeps what it keeps today: the account and the list of the person's kernels.
- **Secrets.** A watcher's credentials are Vault handles; values are filled into the script's environment and nowhere else. The existing redaction applies to event text, readings and logs before any model reads them. The live page card keeps its Vault-filled field masked in the center as in the Room.
- **Untrusted input.** What a watcher reads (a web page, an email) is data. It reaches a subscribed agent as the quoted occurrence, never as instructions, under the existing rules in `docs/PROMPT_INJECTION.md`.
- **Authority.** A private watcher is the person's own code on the person's own machine, run with the kernel's process limits. A subscription acts for its owner, as today. A guest's agent can subscribe to what the Room exposes, as today, and its subscriptions show their creator.
- **Loops.** Ancestry travels with every occurrence, including a watcher's, and a loop is dropped and flagged.

## 12. Later

- **Phone.** A needs-you card or a `high` card makes the kernel ask Cloud for a ping to the person's devices. The ping carries the kernel's id and nothing else. The app wakes, reads the card from the kernel through the relay and shows the notification on the device. Cloud learns that something happened on a kernel, never what.
- **Publishing a source.** A private watcher becomes an App with an "event source" part, the same packaging as AEGS generators and Apps; installers run their own copy with their own Vault. A published source runs other people's code, so it needs the App runtime's containment before this is built.
- **Top-bar bell.** Not yet, as decided.
- **Public sources and billing.** Decision 5 records that a private source is free and a public source anyone can subscribe to needs a paid subscription. Nothing in this plan meters, prices or gates a source on payment; that is designed with the business model.

## 13. Validation

Acceptance runs on the real path: real kernels on two machines, the relay, the Web terminal, real provider agents, the real GitHub source, and pages and feeds under our control. Each cell is a row `notification-center/NC-xx` in the acceptance ledger.

| Cell | Scenario | Passes when |
| --- | --- | --- |
| NC-01 | Inbox across machines: agents in three Rooms on two machines raise a permission, a question, a sign-in page, an App confirmation and a notice | All five are in one Inbox with their origin; the badge counts the four that need the person |
| NC-02 | Close everywhere: the same permission is open in the Room on one client and in the center on another | Allow in the center removes the stripe from the Room on the other client within 2 s and the agent continues. Repeated the other way round. |
| NC-03 | Cross-machine answer: a card from the second machine is answered while the person works in a Room on the first | The agent on the second machine continues; no client reload |
| NC-04 | Live page card from the center: a sign-in code page | The person types the code in the center; the agent continues; the Vault-filled field is masked in every frame the model sees |
| NC-05 | Notify when done: "send me a notification when you are done", then a long task with a context compaction in between | One card when the work ends, with a link; the pane shows "notified you: …"; ordinary turn ends raise no card |
| NC-06 | Notify on PR review: "let me know when you receive the notification to review PR #976", then a kernel restart and a compaction, then real review requests on another PR and on #976, the subscribed agent's kernel being a different kernel from the GitHub source's | The subscription shows its reason and "Only PR #976", picked from the GitHub source's declared options; the occurrence reaches the subscriber's kernel with the PR's number in its `data` and nothing else of the payload; the other PR is filtered out ("Not PR #976") without a wake, and does not use up `once`; #976 wakes the agent, which notifies; the trace shows event, turn, "notified you"; the subscription then removes itself |
| NC-07 | Read, snooze, mute on two clients | State follows to the second client; a snoozed card returns on time; a muted source's cards arrive read; a needs-you card cannot be muted |
| NC-08 | Machines: one machine's connection is cut, then a managed machine is stopped | Cards show under Reconnecting with actions disabled and leave after 10 minutes; no offline machine is listed anywhere; the stopped machine shows Stopped with Start |
| NC-09 | Subscriptions table with agent, workflow, person and workflow-responsible subscribers on two machines | Every subscription is listed once with the right Room, machine, creator, last fired and count; Pause stops delivery and the trace says Paused; Resume delivers the next event |
| NC-10 | Test | The result names who would be woken; nothing is queued, no turn starts, no counter moves |
| NC-11 | Orphan: a source is removed while subscribed | The row says "Source is gone"; Change subscriber and Remove work; no stale row remains |
| NC-12 | Loop and noise: on one kernel, a workflow subscribed to its own exit, then A subscribed to B's exit and B to A's; then A on kernel 1 and B on kernel 2 subscribed to each other's exit; a log that prints 200 matching lines in a minute | On one kernel the self-subscription and the second subscription of A to B to A are each refused when created with "workflow notification cycle", and no row or flag exists for them. Across kernels both subscriptions are created: A's exit starts B, and B's exit, delivered to A, is answered `LoopDropped`: the B-to-A row shows "Loop stopped" after that first drop and the trace says "Loop stopped"; the log source slows itself, one card with Fix, the subscriber is woken at the limit and not 200 times |
| NC-13 | Web page watcher with dry run: from the sentence to Turn on, on a test page we control | No JSON, cron or script is shown; the preview shows the reading, says the page keeps no history and shows the test on a changed copy; changing the real page produces one card and one wake of the subscribed agent |
| NC-14 | Feed watcher | The dry run lists exactly the feed's matching items of the last 24 hours; after Turn on each new item fires once, across a kernel restart |
| NC-15 | Folder watcher | A new file fires once; files added while the kernel was down fire once after restart; nothing fires twice |
| NC-16 | Placement: only machines that sleep, then with an always-on machine | The suggestion shows the real price; Keep costs nothing; Rent goes through the rent confirmation; an agent creating a watcher by tool cannot place it on a paid machine without that click; with an always-on machine present nothing is asked |
| NC-17 | Move a watcher to another kernel while an agent on a third is subscribed | Same source and subscription rows; events before and after the move arrive once each; the credential arrives through the Vault transfer; a move that fails before the commit leaves it running on the old machine, once; the move's activation reply is lost, then both kernels are restarted before step 5: the new machine owns the source, the old copy does not run and is removed after the restart, and the next change in the source fires exactly once; the same with the activation reply delivered but the old kernel down during step 5; and with the new machine unreachable after step 3: the watcher is paused, "Keep it here" resumes the old copy only once the new machine has answered `aborted`, and a `committed` answer completes the move instead |
| NC-18 | Failure is an event: the test page's layout changes | Health changes, one card "can't read the page any more" with Fix; the agent repairs it and shows a dry run; no error text reaches the person |
| NC-19 | Changed meaningfully | A cosmetic change (a timestamp) causes no model call and no event; a real change causes one model call and one event; with the model unreachable the event arrives marked "not judged" |
| NC-20 | Privacy, inspected during NC-03 and NC-13 | Cloud's database and logs contain no title, body, reading or count; the relay carried only encrypted frames |
| NC-21 | An agent creates a watcher by itself with the tools and subscribes itself | No person step; the source and the subscription appear in the center with the agent as creator |
| NC-22 | Workflow as subscriber, with a person as responsible (after #1377) | The source starts a run; a failed run raises a card to the responsible; the responsible is a row in Subscriptions |
| NC-23 | Source-declared options: against a GitHub source and a watcher that declare options; subscribe by the picker and by the agent tool | The picker and `chariox.notification.sources` list exactly the declared options and their types. Picking 976 matches PR 976 and not PR 9760, and the text "976" is refused for a number option. An undeclared option, a wrong type and a repeated option are each refused when the subscription is created, with the source's options listed, and leave no row. An event that lacks the declared field is not delivered to a subscription that picked it. A watcher change that drops a picked option pauses its subscription with "Options changed" and delivers nothing until it is fixed. A subscription with no option picked receives every event. |

## 14. Milestones

Estimates are days of one lane, including review rounds and validation on the real path. M1 to M3 and M4 to M6 are two independent tracks; with two lanes the whole is about three weeks.

| Milestone | Content | Days | Cells |
| --- | --- | --- | --- |
| M1 Inbox | Kernel-wide list of pending interactions for the owner, its change event, the center's Inbox tab, answer from the center, grouping, open in Room, badge, Reconnecting and Stopped | 4 | NC-01 to NC-04, NC-08 |
| M2 Agents notify | `chariox.notify_person`, for-your-information cards, the pane line, read, snooze, mute, the person as subscriber | 3 | NC-05, NC-07 |
| M3 Subscriptions | Kernel-wide list with counters, health and flags; source-declared options (declaration, the picker, exact match, refusal at subscribe time) and the occurrence's `data` of declared fields through the App-event adapter, the stored occurrence and the peer envelope (with the protocol bumps); once; required prompt; pause, change subscriber, remove; test; trace with `turn_ref`; the Subscriptions tab; the Room list replaced by it | 6 | NC-06, NC-09 to NC-12, NC-22, NC-23 |
| M4 Watcher runtime | Package, supervision, cursor, the three modes, declared options, coalescing and rate limit, health as events, dry run, `chariox.watcher.*` | 6 | NC-14, NC-15, NC-21 |
| M5 Sources and creation | The Sources tab, the creation screen with the utility agent, the preview, placement, name and subscribers, Fix and Change | 5 | NC-13, NC-16, NC-18 |
| M6 Move and judge | Move between kernels with the move record, `source_generation` and the commit and status exchange, "changed meaningfully", the push mode | 5 | NC-17, NC-19, NC-20 |
| Later | Phone ping, publishing a source | not estimated | |

Each milestone ships a usable part: after M1 the person already answers everything from one place.

## 15. Decisions and defaults

### Decided by the owner (2026-11-17)

| # | Question | Decision |
| --- | --- | --- |
| 1 | Where in the sidebar? | Rename today's "Notifications" entry in place, after Vault and Extensions. *(P3.)* |
| 2 | A filter on subscriptions? | No filter language. Each source declares the few options a subscriber may pick, each tied to one declared field of its events; the kernel matches the picked values exactly; an undeclared option or a value of another type is refused at subscribe time (section 6.2). *(P1, P2.)* |
| 3 | Does the end of every turn raise a card? | No. Agents notify when asked or when it matters. *(P1.)* |
| 4 | Guests: do cards from someone else's Room appear in your center? | No, in v1 and onward, until the owner decides again. A card in someone else's Room stays in that Room. *(P1.)* |
| 5 | How does a third party's webhook reach a kernel? | Only through a public source, as GitHub does. The push mode serves a local address, as the agent HTTP trigger does, so it covers senders on the machine or its network; this plan opens no new way into a kernel. *(Runtime rule 4: Cloud and the relay carry no runtime traffic in the clear.)* **Business note, recorded only:** a private source is free; a public source that anyone can subscribe to needs a paid subscription. The business model comes later; this plan designs and builds nothing for billing. |
| 6 | Can Test really fire? | No, a dry run only. A real firing is "Check now" on the source. *(P1.)* |

### Decided by the owner (2026-11-18)

| # | Question | Decision |
| --- | --- | --- |
| 8 | Queue or steer? | Per subscription, default queue, set by the agent when it subscribes and changeable from Subscriptions. The 2026-11-03 "always queue" rule is gone. A queued row keeps its Steer button. *(Section 6.1a.)* |
| 9 | Where does the payload go? | In the agent's inbox, for every source. The prompt carries one line: title, source and the inbox position. One path; the payload-in-prompt block is deleted. *(Section 6.1a.)* |

### Technical defaults the plan assumes

7. **Where does coalescing live?** On the source. A subscriber that wants less asks an agent to change the source, or picks an option the source declares. *(P1.)*
8. **Who is the creation agent?** A utility agent in a Room owned by the center, on the watcher's machine, using the person's default provider account there. Its conversation is kept with the source, so Change and Fix continue it. *(P1, P2.)*
9. **Where does a network watcher run when the person has several machines?** An always-on machine they already have, else the machine in use, with the one suggestion of section 7.7. *(P1; money stays a person step, P2.)*
10. **How long do for-your-information cards stay?** 7 days, the existing constant, no setting. *(P1.)*
11. **Does a web page watcher use the kernel browser?** A plain request first; the kernel browser only when the page needs scripts or a sign-in, because it costs memory on the machine. The agent decides. *(P2.)*
12. **Which model judges "changed meaningfully"?** The smallest model of the person's default provider account on that kernel, through the utility-call path, at most once per check. *(P1: nothing to choose.)*
13. **What contains a private watcher?** The kernel's process limits and the person's own machine authority, as for any command their agents run. Sandboxing is required only for published sources. *(P2.)*

## 16. What this replaces

- The sidebar's "Notifications" entry becomes the Notification center.
- `ListWorkflowNotifications {session_id}` and the per-Room list give way to the kernel-wide list filtered to a Room.
- The sentence "a subscription has no options" in `docs/EVENT_TRIGGER_PROTOCOL.md` is true today and is rewritten by the M3 change, not before. After M3 a subscription carries `once`, `paused` and the values picked from the options its source declares (section 6.2), and still has no filter language, event lists, output fields, queue names, delivery modes or retention fields on any request; the document then names those options and `once` as the only things a subscription can carry beyond its source and target.
- Nothing else is removed: workflow notifications, agent triggers, App events and outside agents keep their path and gain the center as their view.
