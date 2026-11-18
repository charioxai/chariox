# Todo reference App

Todos with due times, kept in the App's own state. A due time becomes a kernel-owned wake, so the App
does not stay running to wait; when it falls due the App marks the Todo reminded and emits one `todo_due`
occurrence in the same transaction, and the workflow the person chose with
`/app automation add <installation> reminders todo_due …` runs. An agent sees who it works for: it may
delete only Todos an agent added, and an external `todo_requested` event creates one Todo however often it
is delivered.

The backend is `handler.mjs` (shared by Node tests and the component) built from `app.ts` by the SDK
tools. The page, `view/`, is a native view: it reads the Todos once and follows the App's change feed
(`view_changes`, written in each change's own transaction by the SDK's `createFeed`) through
`window.chariox.changes`, so an agent's change shows in an open window within a second.
`displayName` and `ui/icon.svg` give it the name and logo every client shows.

```sh
node --test examples/apps/todo/test/*.test.mjs
# the real component under Wasmtime (build it first, see the build guide)
node packages/app-sdk-tools/build-js.mjs examples/apps/todo/app.ts "$TMPDIR/todo.wasm"
CHARIOX_TODO_COMPONENT="$TMPDIR/todo.wasm" \
  cargo test -p chariox-app-sdk-tools --features component-drills --test todo-drill
# the signed package with its page, for a drill (writes todo.cxapp and publisher.json)
examples/apps/todo/scripts/build-package.sh "$HOME/.chariox/dev/todo-build"
```

The Node tests inject the state, wake and event imports into `createApp`; the drill runs the built
component with in-memory state fixtures. Build the WASM component with the pinned SDK tools
([build guide](../../../packages/app-sdk-tools/README.md)).
