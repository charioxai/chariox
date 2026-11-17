# Deliver GitHub events to workflows and agents

This page wires GitHub pull-request, review, issue, check and release events to people,
agents, Rooms and workflows. It uses the GitHub App (`examples/apps/github`; its window
lists and reads pull requests and holds the Subscriptions tab, see its README) and the
ordinary kernel pieces: the App's event routes and an `event_based` workflow publication.
The App holds no GitHub credentials; the kernel adds them to the App's GitHub calls.

There is one flow for every kernel. You authorize GitHub once, on one kernel; the
connection then reaches your other kernels through the kernel-to-kernel transfer
path that is being built separately. Until that lands, repeat step 2 on each kernel.
The event generator and the event delivery endpoint are plain kernel configuration
and work the same way everywhere (see "Kernel configuration").

## How an event travels

```
GitHub -> dev.chariox.github generator -> event delivery -> App event route
  -> App handler -> matching subscriptions -> person, agent, Room or workflow
```

- The generator turns a GitHub webhook into one event per connection scope
  (`owner/repository`) and event type.
- The App asks the kernel for one route per repository and event type that its
  subscriptions need, and removes the routes nothing needs any more (`eventRouting`
  in the App's declarations). You never create a route by hand. The kernel accepts
  a delivery and acknowledges it before the App handler runs, and a repeated
  delivery of the same event never runs the handler twice on that route.
- The handler matches the event against every enabled subscription (scope, event
  kind, filters), keeps one record of the outcome per subscription (delivered,
  filtered with the reason, or failed) and sends the notification to each target
  once. The person gets a host notification; an agent, a Room's agent or a workflow
  gets one directed `github_notification` occurrence whose text is the prompt its
  turn or run starts with.

A subscription names its events with kinds: `pull_request.opened`, `.updated`,
`.review_requested`, `.review_submitted`, `.merged`, `.closed`, `.ready_for_review`,
`.reopened`, `.labeled`, `.comment`, `check.completed`, `ci.finished`,
`workflow_run.completed`, `issue.opened`, `.closed`, `.labeled`, `.comment`,
`release.published`, `push` and `mention`. `src/event-types.mjs` of the App maps each
kind to the generator types it needs.

The generator normalizes `pull_request.{closed, ready_for_review,
reopened, labeled}`, `pull_request_review.submitted`,
`pull_request_review_comment.created`, `issue_comment.created`, `issues.{closed, labeled}`,
`check_run.completed`, `check_suite.completed`, `release.published` and `push`. A
GitHub App installation delivers the new types only after its owner accepts the updated
permissions (Checks and Contents: read) on GitHub; the pull request, review requested, issue
and workflow run types are unaffected.
Signed payload fixtures for every type are in `docs/fixtures/event-generators/github/`.

Every event's `metadata.chariox` holds facts GitHub only gives in nested lists, so a
`--filter-json` such as `{"chariox.labels": "needs-review"}` works without calling GitHub:
`labels` (names), `merged`, `pr_numbers` (the pull requests of a check, workflow run,
comment or pull request event), `body_mentions` (`@logins` of the comment, review, release,
pull request or issue body; team mentions and addresses excluded), and when present
`base_ref`, `branch` (base of a pull request, head of a check or workflow run, the pushed
branch), `head_sha`, `sender_login`, and for pushes `is_default_branch_push`.

The notification's `data` carries the subscription (`id`, `name`), the event kind, the repository, the
pull request or issue number and the URL, and the GitHub facts the kind is about (the head commit,
the check conclusion). Its text is the prompt.

## Kernel configuration

Whoever operates the kernel sets these; they are the same for every kernel. Without
them the kernel has no event delivery and the steps below cannot work.

| Setting | Meaning |
|---|---|
| `CHARIOX_AEDS_URL`, `CHARIOX_AEDS_TOKEN` | the event delivery endpoint and its token |
| `CHARIOX_EVENT_ENVIRONMENT_ID` | names this kernel's deliveries; use a different value per kernel |
| `CHARIOX_EVENT_REGISTRY_URL` | the generator registry |
| `CHARIOX_AEGS_MANAGEMENT_TARGETS_JSON` or `_FILE` | the generator's management endpoint |

## Steps

The commands are TUI slash commands; the `chariox app ...` CLI takes the same verbs
without the leading slash. Example names below (`github`, `review-workflow`,
`pr-updated`) are yours to choose.

### 1. Install the App

Build and install the App from `examples/apps/github` as described in
[Kernel App development](APPS_DEV_PUSH.md), for example `/app dev "examples/apps/github"`,
or install a signed release with `/app install "github.cxapp"`. A release
signed by a publisher key the kernel does not trust yet needs the owner to trust that
key first with `/app publisher enroll "publisher.json"`. `/app list` shows the
installation ID; the examples below use `github`.

### 2. Authorize GitHub on the kernel (owner consent)

GitHub access is the owner's consent, never an agent's:

```
/notifications connect dev.chariox.github
```

Open the URL the kernel prints, approve the GitHub App for the repositories you want,
then check the result with `/notifications authorization <authorization-id>` and
`/notifications connections`. The connection ID printed there is used in step 4.
`/notifications connection resources <connection-id>` lists the repositories (the
connection scopes, `owner/repository`).

### 3. Create a subscription

In the App's window, open the Subscriptions tab and choose New subscription: a name, the
repositories (or an organization, or all), the events, optional filters (branches, authors,
labels, paths, draft, check conclusion; patterns with `*`, a leading `!` leaves a name out) and
who is told. The tab lists every subscription with its state (Active, Paused, Disabled after five
failed deliveries in a row), pauses, resumes, edits and deletes it, and shows its last
deliveries and the filtered ones with the reason. A banner offers to connect GitHub events
when step 2 has not been done.

An agent makes its own subscription with the App's tools: `create_subscription`,
`update_subscription`, `pause_subscription`, `resume_subscription`, `delete_subscription`,
`list_subscriptions`, `list_deliveries`, `get_event_sources`. An agent subscribes itself without
asking. To tell the person, another agent, a Room or a workflow it uses
`create_subscription_for_others`, which asks the person first.

A workflow target needs an `event_based` publication on the workflow's entry endpoint
(`/workflow trigger create <workflow-id> <endpoint-id> --kind event_based`); the subscription
names the workflow's session, publication and optionally its queue, and the App has the kernel
bind the automation.

**One pull request, once.** `watch_pull_request` (and a subscription with `once`) ends after
its first delivery or at its expiry, whichever comes first: 4 hours by default, at most 7
days. `ci.finished` fires when the head commit has checks and none is pending, or after 15
minutes with none. A merged pull request delivers `pull_request.merged` and not also
`pull_request.closed`.

### 4. Check

`/notifications connection test <connection-id>` sends a test occurrence through the
normal generator and delivery path. The subscription's last deliveries then show it, and
`/app inbox list github` shows the route's delivered count.

## Follow the events from an agent outside Chariox

An agent that runs outside Chariox (a Claude Code or Codex CLI session) cannot be an
automation target, but it registers for the notifications itself. It asks for a Kernel
Access grant (`chariox access request --until-exit`; the owner approves it), then follows
either the App's events directly or the workflow an event starts.

### Directly: the App's event

Every route the App makes for a subscription is a notification source of its own, named
for the App's incoming event and the route's repository: `pull_request_updated@<owner>/<repository>`,
shown as "Pull request updated (<owner>/<repository>)". A route with no connection scope is named
for the event alone. A source exists while the App is installed and its route exists, that is while an enabled
subscription needs it; an event kind with no route has no source, because no event of that kind
can arrive.

```
chariox events sources                                       # lists one source per route, kind app_event
chariox events subscribe --source "pull_request_updated@<owner>/<repository>"
chariox events wait                                          # returns when an event arrives
chariox events ack <cursor>
chariox events unsubscribe --source "pull_request_updated@<owner>/<repository>"   # or --source all
```

The notification arrives when the kernel accepts the event, before the App's handler runs, and its
text is the event text (`Handle GitHub pull_request.synchronize for <owner>/<repository> #<number>
at head <sha>`), before any subscription filter is applied. Following needs the same grant as every other source and adds no authority. An
event that arrives while nobody follows its source is kept for the App only; a later follower
receives events from the moment it subscribes. A restart keeps the follow for a grant that lasts
until its holder exits, and an unread notification is kept for 7 days. At most 64 sources per grant.

### Through the workflow the event starts

An agent can also follow the workflow a subscription starts (step 3):

```
chariox events sources                      # lists "<workflow> run finished" for each workflow
chariox events subscribe --source "<workflow> run finished"
chariox events wait                         # returns at once when a record is pending
chariox events ack <cursor>
```

The record's output is JSON: `status` (`Completed` or `Failed`), `run_id`, `workflow_id`
and `prompt`, which is the event text, plus `failure` for a failed run. The notification
arrives when the run ends, not when the event arrives; a stopped run emits nothing. Use this
path to be told what a workflow did with an event, and the direct path to be told that the
event arrived.

## Several kernels

A route ID is unique within one installation and the App derives it from the event type and the
repository (`pull_request.opened@<owner>/<repository>`).

Event delivery identifies a route by `binding_id`, which is
`sha256(owner, installation ID, route ID)`. A second kernel that makes the same
owner, installation ID and route ID as the first receives nothing, and the only trace
is a log line, `event route conflicts with another route`. Give the second kernel's App a
different installation ID. After the kernel-to-kernel connection transfer arrives, each
kernel still keeps its own subscriptions and routes, so the same rule applies.

Duplicate delivery to two kernels is expected: each runs its own targets once.

## Limits worth knowing

- Delivery to a workflow or agent is at least once to the App and once per subscription
  after that; do not rely on ordering between events.
- An event payload and a prompt are each at most 64 KiB.
- At most 100 subscriptions, 200 routes, 200 automations and the last 200 deliveries per App.
  An installation has a route limit of its own; a subscription that needs more is refused with the
  number it needs.
- Deleting a subscription removes the routes and the automation nothing else uses. A workflow
  publication that is removed makes each delivery to it fail; five failures in a row disable the
  subscription, which the window shows as "Disabled after failures" and whose last deliveries say why.
