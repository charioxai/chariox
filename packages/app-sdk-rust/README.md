# Rust App SDK

The bindings come directly from `packages/app-wit/wit`, the async R1 contract.
Use the checked-in `template` as a source-based Todo backend. `Guest` is the
exported handlers trait; `bindings::export!(App with_types_in
chariox_app_sdk::bindings)` emits the component exports. `cancellable(cancel,
work)` drops pending work and owned streams when the cancellation future wins.
`get_json` retains the host state version for compare-and-swap transactions.

Build and prepare the output using [the shared compiler instructions](../app-sdk-tools/README.md).
This SDK currently builds from the monorepo alongside its canonical WIT; it is
not a standalone crates.io distribution. Keep the SDK and WIT from the same
revision. The two Todo operations use durable state and optimistic checks; a
concurrent write may return the host's conflict error instead of losing data.


## Name and icon

`displayName` and `icon` in `app.json` give the App the name and logo that every client shows
(the Desktop tile, lists). The template's `bundle/ui/icon.svg` is the example: an original SVG
of at most 32 KiB (or a PNG up to 512 by 512) listed in the bundle; `pack` refuses scripts and
outside references. An App without them is shown under its app id with a letter tile.

## Serving a view to an agent

`view::View::declare(include_str!("ui-actions.json"))` takes the signed `ui-actions` document once.
Call `register()` in `Guest::register` and `serve(tool, input, state)` first in `call_tool`: it
answers the view's registration, state and (after `with_changes()`) change feed, and returns `None`
for the App's own tools. Commit a change with `view::transact_with_change(tx, delta)`; it adds the
change to the App's transaction, keeps the last 128 and refuses a change over 8 KiB. The tools it
registers (`view_actions`, `view_state`, `view_changes`) must be listed in the manifest `tools`.
See `packages/app-view-host/fixture/guest`.

## Browser UI handlers

This SDK dispatches backend WIT handlers. Browser UI actions receive
`(input, caller)` and use `await caller.call(method, input)` for backend work
belonging to that invocation. Ambient `chariox.call` always acts as the human.
Register async forms with `chariox.form(element, async (input, caller) => {...})`;
annotated forms without registration are refused. See the
[view caller and form contract](../app-wit/README.md#view-callers-and-forms).
