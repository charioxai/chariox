# Agent Apps External Integration Concept

This document extends `docs/AGENT_APPS_CONCEPT.md`. It describes how Agent Apps
can be integrated into existing web and mobile applications without assuming
that every app is deployed through Chariox-owned containers.

It is a concept document, not an implementation plan.

## Core Constraint

Chariox has a specific runtime shape:

- the kernel is the workflow/runtime authority
- providers run through provider-native CLIs, adapters, or local servers
- workflow sessions, queues, traces, artifacts, and hidden publication runtimes
  are owned below clients
- Cloud is a control plane and deployment surface, not a replacement runtime
- provider credentials must remain provider-owned and runtime-local

Therefore, an external web app integration must always answer this question:

```text
Where does the Chariox runtime live?
```

A package installed into an arbitrary web app can wrap routes and call Chariox,
but it cannot universally run provider CLIs, durable queues, workflow replicas,
and writable overlays inside every hosting environment.

## Integration Modes

### Chariox-Hosted Agent App

In this mode, Chariox hosts the app-facing runtime.

```text
browser
  -> Chariox public URL
  -> Agent App Gateway / publication server
  -> kernel
  -> provider CLIs
  -> app assets, overlays, workflow outputs
```

The deployment can include:

- publication server
- kernel
- workflow package
- provider binaries
- app assets or build output
- overlay store
- artifact store
- queue and replica pool
- generated app-action tools

This is the most complete mode. It supports prompt-in-URL HTTP GET, API/SSE,
WebSocket, MCP-style access, generated HTML, dynamic web-app views, traces,
overlays, app actions, replica pools, and persistent patches when policy allows.

The tradeoff is that the app route being mediated by the agent is served through
the Chariox deployment or a Chariox-controlled ingress.

### Existing App With Chariox Sidecar

In this mode, the user's app stays where it is, and a Chariox runtime runs
beside it.

```text
browser
  -> existing app
  -> wrapped route middleware
  -> local Chariox sidecar runtime
  -> kernel/provider/workflow
  -> response effects back to app/browser
```

This is the natural mode for:

- Docker Compose
- Kubernetes
- a VM
- a long-running Node/Rails/Django/etc. server
- self-hosted deployments
- platforms that allow sidecars or private service networking

The sidecar can run provider CLIs and keep durable workflow state. The existing
app only needs middleware or route handlers that call the sidecar for wrapped
routes.

### Existing App With Remote Chariox Runtime

In this mode, the app cannot or should not run the Chariox runtime locally. The
app calls a remote runtime instead.

```text
browser
  -> existing app or serverless route
  -> Chariox integration middleware
  -> remote Chariox Agent App Runtime
  -> kernel/provider/workflow
  -> streamed/final response back
```

This is the realistic mode for Vercel-style or serverless deployments. The app
package can wrap routes and stream responses, but provider execution happens in
a remote runtime: Chariox Cloud, a customer VM, a customer container, or another
self-hosted runner.

This avoids pretending that every serverless request handler can run provider
CLIs, hold long-lived workflow queues, store overlays, or keep WebSocket/SSE
sessions open reliably.

### Embedded Development Mode

For local development, a framework package can start or connect to a local
Chariox runtime.

```text
developer app server
  -> local wrapped route
  -> local Agent App Runtime
  -> local kernel/provider/workflow
```

This is useful for development and testing. It should not be treated as a
universal production deployment model.

## Agent App Gateway

The independent software/service that simplifies this should be an Agent App
Gateway.

The gateway can be distributed as:

- a container
- a standalone binary
- a local development service
- a Chariox-hosted service
- a sidecar process next to an existing app

Its responsibilities are:

- match wrapped routes
- extract prompts from path, query, body, headers, or WebSocket messages
- return a streaming shell immediately for browser routes
- call the published workflow runtime
- stream queued, started, partial, trace, final, and error events
- serve generated overlays and artifacts
- resolve files through overlay, patch, and base app layers
- proxy normal routes unchanged when configured as a fronting gateway
- enforce endpoint manipulation policy
- expose app actions to workflows through generated tools/MCP
- manage invocation and session overlays
- dispatch requests across workflow replica pools
- record deployment status and logs

The gateway is not a provider runtime by itself. It needs a kernel-backed
runtime either embedded in the same process/container or reachable as a service.

## Web App Integration Package

Existing web apps should integrate through a small framework package plus a
manifest.

Example manifest:

```json
{
  "routes": [
    {
      "path": "/add/*",
      "workflow": "cart-agent",
      "endpoint": "cart-entry",
      "prompt_source": "path_tail",
      "response": "streaming_shell",
      "manipulation": "state_and_overlay"
    }
  ],
  "actions": {
    "cart.search": {
      "method": "POST",
      "url": "/internal/cart/search"
    },
    "cart.add": {
      "method": "POST",
      "url": "/internal/cart/add"
    }
  }
}
```

The package should provide:

- route middleware for wrapped routes
- a client for local, sidecar, or remote Agent App Runtime
- helpers for browser streaming shells
- helpers for proxying overlay assets
- action registration for generated MCP/tools
- local commands for development and packaging
- deployment commands or metadata for Cloud/sidecar/container modes

The package should not contain provider credentials or Chariox account
credentials.

## Route Wrapping

Normal routes continue to behave normally. Wrapped routes are mediated by an
Chariox workflow endpoint.

Example:

```text
GET /add/1 kg bananas, 2 bottles of 1l Coca-Cola, and a bag of chips
```

The wrapper:

1. matches `/add/*`
2. extracts the path tail as the prompt
3. opens a workflow invocation through the Agent App Runtime
4. returns a streaming shell to the browser
5. receives partials, traces, final output, overlays, or redirects
6. serves the final app state, such as checkout

The route can both take actions and customize rendering. These are not separate
features; they are response effects from the same workflow invocation.

## App Actions

For app behavior such as adding products to a cart, updating records, searching
a database, or preparing checkout, the workflow should use declared app actions.

Those actions can be exposed through a generated MCP server or equivalent
kernel-owned tool surface.

The developer declares what actions exist. The endpoint policy decides which
actions a workflow can use.

This is better than generic browser automation for ordinary app behavior
because it is typed, auditable, faster, and enforceable below prompt text.

Browser automation can still exist for advanced cases, but it should not be the
default app integration mechanism.

## Source, Assets, And Overlays

Agent Apps should not require copying a full app for every invocation. The
serving layer should resolve content in this order:

```text
session or invocation overlay
  then persistent patches
  then base app files or proxied app route
```

What can be modified depends on what the Agent App Runtime can access.

If it has built assets:

- it can overlay HTML, CSS, JavaScript, images, and generated files
- it can serve generated app views for wrapped routes
- it does not need to rebuild the source app

If it has source code and build tools:

- it can patch source
- it can rebuild
- it can serve the rebuilt output
- this is heavier and should be explicit

If the app is deployed on a platform where Chariox only has a remote runtime:

- Chariox cannot directly mutate that platform's deployed source
- Chariox can return generated overlays for wrapped routes
- persistent source changes require a repository/deployment integration, such
  as commit, pull request, platform API, or redeploy flow

## Persistent Patch

Persistent patch remains useful but powerful. It means future users see the
changed behavior.

Examples:

- temporary production banner
- emergency copy/style patch
- generated A/B variant
- short-lived hotfix before a normal deploy

Persistent patch should require:

- endpoint policy that allows it
- strong role/auth checks
- diff/preview
- audit log
- rollback
- preferably expiry

It should not be available to arbitrary public users.

## Deployment And Chariox Flow

A complete deployment flow should look like this:

```text
draft workflow
  -> publish endpoint
  -> package workflow snapshot, requirements, trace policy, route manifest,
     action manifest, and optional app assets
  -> deploy Agent App Runtime/Gateway
  -> register deployment in Chariox Cloud
  -> expose public URL or sidecar/remote runtime URL
```

For Chariox-hosted deployments, the package can include app assets and the
gateway/runtime can serve the app route directly.

For sidecar deployments, the package configures the sidecar and the existing
app middleware talks to it.

For remote-runtime deployments, the existing app keeps serving itself and calls
the remote Agent App Runtime only for wrapped routes.

## Serverless And Platform Limits

Serverless platforms are important, but they are not a good place to run the
full Chariox runtime directly.

Common limitations:

- request timeouts
- no durable long-running provider process
- limited or ephemeral filesystem
- uncertain WebSocket/SSE support depending on platform
- no local provider CLI login state
- cold starts
- limited background processing

Therefore, serverless integration should be route wrapping plus remote runtime,
not provider execution inside each serverless function.

## Mobile Integration

Mobile apps cannot receive arbitrary modified compiled native source at request
time. iOS and Android apps are compiled, signed, and shipped.

Agent Apps still apply to mobile through several practical modes.

### Native App Actions

The app exposes actions to the workflow.

```text
native prompt
  -> workflow endpoint
  -> app-action tool bridge
  -> native app state update
```

This supports acting on behalf of the user:

- add items to cart
- search data
- prepare a checkout
- update app state
- trigger existing backend operations

### Server-Driven Native UI

The workflow returns structured data or a UI schema. The native app renders it
using predefined components.

This is less flexible than arbitrary HTML/JS, but it feels native and can stay
inside platform expectations.

### Runtime HTML Through WebView

A native app can load an Agent App surface in a WebView.

```text
native shell
  -> WebView
  -> Agent App URL
  -> workflow-generated HTML/CSS/JS/assets
```

This is the closest mobile equivalent to mutable web surfaces. It supports
generated dashboards, dynamic reports, generated forms, admin views, and other
runtime-created UI.

The native app can optionally expose a bridge so the WebView can call selected
native capabilities, subject to endpoint policy.

### Remote Config And Asset Overlays

The workflow can generate copy, theme values, images, or structured layout data
that the app loads at runtime.

This is safer and narrower than arbitrary source mutation.

### Native Source Mutation Limit

Arbitrary native source mutation at request time is not realistic. A workflow
can generate a patch or pull request for native source, but applying it to users
requires the normal build, signing, review, and release process.

For runtime behavior, mobile should use actions, server-driven UI, remote
config, assets, or WebViews.

## What Is Universal

The universal Agent App abstraction is:

```text
existing app route or app action
  + wrapped Chariox workflow endpoint
  + Agent App Runtime/Gateway
```

The runtime location varies by deployment mode. The workflow output model can
remain common across modes: response, effects, overlays, actions, traces, and
artifacts.

## What Is Not Universal

The following cannot be guaranteed across every external app host:

- provider execution inside the app host
- direct mutation of deployed source code
- durable local queues inside serverless request handlers
- local provider credentials in frontend or serverless code
- unrestricted filesystem overlays
- arbitrary native mobile source mutation at runtime

These are deployment capabilities, not core Agent App assumptions.

