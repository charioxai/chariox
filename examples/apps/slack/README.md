# Slack reference App

Slack events reach the App only through the generic path: the owner's Slack connection at the Slack event
generator, an App inbox route per event type (`/app inbox add … --connection`) and this App's signed incoming
events (`mentioned`, `channel_message`, `reaction_added`). It keeps the latest 100 notifications in its own state
and hands each new one to the workflow the owner chose, through the App automation `notifications`. Replies and
reactions go back through the Slack connection the owner granted the App (`/app connection grant`), with the
generator's reply context: the App never sees a Slack token.

The backend is `handler.mjs` (shared by Node tests and the component) built from `app.ts` by the SDK tools. The
page, `view/`, is a native view: it reads the notifications once and reads them again when the App's change feed
(`view_changes`) says one arrived or was cleared. It has no form (the sandbox submits none): Reply answers its
button and Enter.

```sh
node --test examples/apps/slack/test/*.test.mjs
# the signed package with its page, for a drill (writes slack.cxapp and publisher.json)
examples/apps/slack/scripts/build-package.sh "$HOME/.chariox/dev/slack-build"
```

The Node tests inject the state, event and connection imports into `createApp`. Build the WASM component with the
pinned SDK tools ([build guide](../../../packages/app-sdk-tools/README.md)).
