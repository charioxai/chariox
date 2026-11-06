# GitHub events

Delivers GitHub events to any number of workflows and agents. The App declares the
generator's pull request opened, pull request updated, review requested, issue
opened and workflow run completed events as incoming events, and one outgoing event
per type. Its handler emits an accepted delivery to every automation named in the
App setting. It holds no GitHub credentials and makes no GitHub calls.

The setting maps each outgoing event (for example
`pull_request_updated_notification`) to automation names. Edit it in the App's view
or with the `configure_automations` tool; `get_automations` reads it. An event whose
setting was never saved fails in the handler; an empty list ignores the event.

Steps, from installing the App to the first delivery:
[Deliver GitHub events to workflows and agents](../../../docs/GITHUB_EVENTS_TO_WORKFLOWS.md).

```sh
node --test examples/apps/github-events/test/*.test.mjs
cargo test -p chariox-app-package --test developer github_events
```

The Node tests inject the state and event imports into `createApp`; the Rust test
packs and validates the real bundle with a stand-in component. The kernel side is
covered by `app_event_path_tests::event_path_one_delivery_runs_a_workflow_and_an_agent_once`.
Build the WASM component with the pinned SDK tools
([build guide](../../../packages/app-sdk-tools/README.md)).
