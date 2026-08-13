# Agent Apps Concept

This document captures the product and architecture concept for Agent Apps. It
is not an implementation plan. It records the model that should guide later
workflow publication, deployment, web-app, and mobile integration work.

## Summary

An Agent App is a web or native application with one or more routes or actions
mediated by a Chariox workflow endpoint.

Normal routes continue to behave like normal application routes. Wrapped routes
invoke a published workflow before, during, or after serving the route. The
workflow can produce response effects such as generated output, file overlays,
app-state changes, redirects, streamed progress, app actions, or persistent
patches within the endpoint's manipulation policy.

The kernel remains the workflow authority. The publication server interprets
workflow outputs and serves app assets. The app developer chooses which routes
are agent-mediated and how much authority those endpoints have.

## Core Model

An Agent App deployment contains:

- a base app, such as existing HTML/CSS/JS/assets or a server-rendered app
- one or more published workflows
- wrapped routes that map app requests to workflow endpoints
- optional app actions exposed to the workflow as tools
- a publication server that serves normal routes and applies workflow effects
- a runtime scheduler that dispatches invocations to workflow replicas

Conceptually:

```text
request
  -> publication/app server
  -> normal route: serve app normally
  -> wrapped route: invoke workflow endpoint
      -> workflow emits partial/final response effects
      -> server serves/streams/redirects using those effects
```

This should remain an extension of workflow outputs and publication behavior,
not a separate kernel-owned application framework.

## Generalized Workflow Output

Agent Apps should minimize the number of output `kind` values. The preferred
direction is one general response output shape rather than many special cases.

Example:

```json
{
  "kind": "response",
  "response": {
    "mode": "serve",
    "entry": "/checkout"
  },
  "effects": {
    "overlay": [
      {
        "path": "/checkout.html",
        "mime_type": "text/html",
        "content": "<!doctype html>..."
      }
    ],
    "state": {
      "cart_id": "cart_123"
    }
  }
}
```

The `response.mode` determines how the publication server answers the caller.
Expected modes include:

- `serve`: serve an app route or asset using base app plus effects
- `html`: render inline HTML as a complete response
- `json`: return a JSON body
- `redirect`: redirect to another app route
- `status`: return progress/status without completing the app view

Effects are optional. Expected effects include:

- `overlay`: generated or modified files that shadow base app files
- `state`: app/session state produced by the workflow
- `actions`: app actions the workflow performed or requests the runtime to
  perform
- `artifacts`: generated assets or file references
- `patch`: persistent source/app patch, when endpoint policy allows it

The kernel should store and expose this output as structured workflow output.
It should not interpret web response semantics. Interpretation belongs to the
publication server.

## Base App, Overlays, And Patches

The base app is the developer-provided application source or built assets.
Agent Apps should not require a full copy of the app for every invocation.

The serving layer should resolve files in this order:

```text
session/invocation overlay
  then persistent patches
  then base app files
```

### Ephemeral Overlay

An ephemeral overlay is scoped to one invocation or run. It is useful for:

- one-off generated dashboards
- request-specific reports
- temporary custom views
- prompt-specific app variants that should not affect other users

Example:

```text
GET /dashboard/vibrant-for-this-query
```

The workflow can generate an overlay for this run only. When the run expires,
the overlay can be garbage-collected.

### Session Overlay

A session overlay is scoped to one browser/user/app session. It is useful for:

- multi-page personalized flows
- shopping cart preparation plus customized checkout rendering
- "make this app look like X for my session" interactions
- temporary generated pages/assets that should persist while the user navigates

The overlay affects only that session, not other users.

### Persistent Patch

A persistent patch changes the deployed app behavior for future users. This is a
powerful developer/admin capability and must be explicit. It can support:

- emergency production copy/style patches
- temporary banners or feature hiding
- generated A/B-test variants
- short-lived production hotfixes before a normal deploy

Persistent patch should require endpoint policy that allows it, strong
authorization, audit logs, diff/preview, rollback, and preferably expiry. It is
not a default public-user capability.

## Wrapped Routes

A wrapped route maps an application route to a workflow endpoint. The path tail,
query, body, headers, or WebSocket message can become workflow input depending
on transport.

Example:

```json
{
  "route": "/add/*",
  "endpoint_id": "cart-agent",
  "prompt_source": "path_tail",
  "response": "streaming_shell",
  "serve_after": "/checkout",
  "manipulation": {
    "level": "state_and_overlay",
    "allowed_actions": ["cart.search", "cart.add", "cart.prepare_checkout"],
    "allowed_paths": ["/pages/checkout/**", "/assets/generated/**"],
    "protected_paths": ["/auth/**", "/payments/**"]
  }
}
```

Request:

```text
GET /add/1 kg bananas, 2 bottles of 1l Coca-Cola, and a bag of chips
```

Flow:

```text
1. Server receives request.
2. Route is workflow-wrapped.
3. Prompt is parsed from the path tail.
4. Workflow runs.
5. Workflow can call app actions and produce overlays/state.
6. Server serves or redirects to the checkout view using those effects.
```

The same wrapped route can both take actions and customize what is rendered.
These are modes inside one Agent App model, not separate use cases.

## App Actions And Generated MCP

App actions should be exposed to workflows through a generated MCP server or
equivalent kernel-owned tool surface.

The developer declares actions:

```json
{
  "actions": {
    "cart.search": {
      "input_schema": {
        "type": "object",
        "properties": {
          "query": { "type": "string" }
        },
        "required": ["query"]
      },
      "transport": {
        "kind": "http",
        "method": "POST",
        "url": "http://app.local/internal/cart/search"
      }
    },
    "cart.add": {
      "input_schema": {
        "type": "object"
      },
      "transport": {
        "kind": "http",
        "method": "POST",
        "url": "http://app.local/internal/cart/add"
      }
    }
  }
}
```

The runtime exposes only the actions allowed by the wrapped endpoint policy.
The generated MCP/tool layer validates inputs, enforces endpoint policy, and
audits action calls. If an endpoint is not allowed to call `payments.refund`,
that tool is not exposed or returns a policy error.

This is preferred over generic browser automation for ordinary application
state changes. Browser automation can exist as an advanced capability, but it
is slower, more fragile, and riskier for destructive flows.

## Streaming Shell And Partial Outputs

Browser-facing wrapped routes should be able to return an immediate runtime
shell before the workflow starts or while it is queued.

This shell is owned by the publication server, not by the workflow. It can show:

- queued status
- running status
- partial workflow outputs
- exposed traces if configured
- final response once available

The workflow may still emit partial outputs to update the shell. This supports
progressive experiences such as:

- "searching products"
- "added bananas"
- "preparing checkout"
- preview dashboard sections
- intermediate generated files/assets

The final output is a `kind: "response"` payload that the shell applies by
serving, redirecting, rendering HTML, or updating the current view.

For fast endpoints, blocking response mode can still exist. For browser GET
routes that may involve an agent, `streaming_shell` should be the default
because provider startup, queueing, and agent work can exceed normal page-load
expectations.

## Endpoint Manipulation Policy

Each wrapped endpoint should declare the authority it gives the workflow. This
is where app-specific guardrails belong.

Possible manipulation levels:

- `none`: workflow returns JSON/text/status only
- `state`: workflow can call allowed app actions but cannot change served files
- `overlay`: workflow can generate or replace allowed files for this run/session
- `state_and_overlay`: workflow can call actions and generate overlays
- `full_ephemeral`: workflow can rewrite any non-protected served artifact for
  this invocation/session
- `persistent_patch`: workflow can change future served app behavior, subject
  to strong authorization and audit controls

Policy dimensions:

- allowed read paths
- allowed overlay paths
- protected paths
- allowed app actions
- maximum generated asset size
- script/build allowance
- frontend sandbox/CSP profile
- external network allowance
- output scope: invocation, session, persistent
- required role or auth condition

The policy must be enforced below prompt instructions. The agent should not be
able to bypass it by ignoring instructions.

## Guardrail Enforcement Layers

Guardrails should exist at multiple layers:

1. Publication server:
   - path allow/deny checks
   - MIME and size validation
   - overlay and persistent patch enforcement
   - response mode enforcement

2. Kernel/tool layer:
   - generated app-action MCP exposes only allowed tools
   - schemas are validated
   - denied actions return policy errors
   - calls are audited

3. Filesystem/workspace layer:
   - base app can be mounted read-only
   - workflow writes go through an overlay or patch API
   - direct writes outside allowed roots are blocked where practical

4. Container/OS layer:
   - unprivileged user
   - isolated writable directories
   - no host filesystem access
   - future network egress constraints if required

The first implementation should not over-design every security feature, but the
model must make it possible to enforce policy outside the prompt.

## Scaling, Replicas, And Queueing

Agent Apps should not require one container per workflow. A deployment should be
able to host multiple published workflows and multiple replicas of a workflow.

Example:

```text
deployment container
  kernel process
  publication server
  workflow runtime pools
    dashboard workflow: replica 1, replica 2, replica 3
    cart workflow: replica 1, replica 2
```

A workflow replica is a separate hidden runtime session materialized from the
same publication snapshot. It has separate provider processes, active run
state, and overlay/session state. The existing no-concurrency guarantee still
holds inside one replica.

The deployment scheduler should dispatch requests across a workflow's replica
pool:

```text
incoming request
  -> workflow pool scheduler
  -> idle replica if available
  -> otherwise queue according to policy
```

The queue should be pool-aware so work is not stranded behind a busy replica
while another replica is idle.

### User Ordering

For app users, a useful policy is per-caller ordering on top of the pool queue:

- requests from the same caller/session are processed in order
- a later request from that caller does not start before the caller's previous
  request finishes
- if that caller's bound replica is still busy with the caller's previous
  request, the next request waits for that replica
- if the bound replica is busy with someone else, the request may dispatch to
  any idle replica
- when all replicas are busy, the request waits in the pool queue

This is close to keyed FIFO or session-affine scheduling. It is related to
queueing concepts such as per-key ordering, session affinity, and fair
scheduling. It is not arbitrary: it preserves a user's causal sequence while
allowing the deployment to use idle replicas for other users.

The draft-workflow queue model can still exist inside each materialized
runtime. Deployed Agent Apps add a deployment-level scheduler before requests
enter a runtime replica. For deployed workflows, the product-visible queue is
the pool queue; the kernel-owned workflow queues remain the per-replica
execution mechanism.

Queue policy should eventually expose:

- replica count
- maximum queue depth
- per-caller ordering on/off
- queue timeout
- overflow behavior: queue, reject, redirect, or future scale-out

## Roles And Authorization

Roles should be first-class at the wrapped endpoint/publication boundary, not
only at deployment time.

Draft workflows can define desired endpoint roles as design-time metadata, but
they are advisory until publication. Published workflows freeze the endpoint
role requirements into the publication artifact. Deployments bind those roles
to an auth provider or leave them unmanaged/public.

This separation keeps the model portable:

```text
draft endpoint role metadata
  -> published endpoint policy
  -> deployment auth binding
```

Example:

```json
{
  "route": "/admin/patch/*",
  "endpoint_id": "admin-patch-agent",
  "required_role": "admin",
  "manipulation": {
    "level": "persistent_patch"
  }
}
```

Chariox does not need to own all auth providers. It should preserve endpoint
role requirements and make them enforceable by deployment infrastructure,
Cloud-hosted auth, or a self-hosted wrapper.

## External Web App Integration

Agent Apps should not require every user to deploy through Chariox containers.
There should be an integration path for existing web app deployments such as
Vercel, Render, Fly, Kubernetes, or a user's own server.

The external integration package would provide:

- route middleware for wrapped routes
- a client to a Chariox kernel/publication runtime
- static asset overlay resolution
- streaming shell helpers
- generated app-action MCP/tool registration
- local/dev commands to publish/export required workflow packages

However, provider execution still requires a Chariox kernel and provider
runtime somewhere. Many serverless environments cannot run long-lived provider
CLI processes inside request handlers. Therefore external integration likely
has two modes:

1. Sidecar/service mode:
   - app stays deployed where it is
   - Chariox kernel/publication runtime runs as a sidecar, VM, container, or
     hosted Chariox service
   - web app middleware calls that runtime

2. Embedded development mode:
   - local development can install packages and run Chariox locally
   - useful for testing, but not suitable for all production hosts

For Vercel-style platforms, the likely production path is middleware plus a
remote Chariox runtime, not bundling provider CLIs into the Vercel function.

Provider credentials should remain provider-owned and runtime-local. The
external app package should not include provider credentials.

The external integration model, including sidecars, remote runtimes, the Agent
App Gateway, existing web app middleware, platform limits, and mobile integration
surfaces, is detailed in `docs/AGENT_APPS_EXTERNAL_INTEGRATION.md`.

## Mobile Apps

Mobile apps cannot receive arbitrary modified compiled source at request time in
the same way web apps can receive modified HTML/CSS/JS assets.

Agent App concepts still apply to mobile through:

- app actions exposed as tools
- prompt or voice areas that invoke workflow endpoints
- workflow-backed state changes
- generated data/views rendered by native components
- server-driven UI within predefined native renderers
- embedded web views for dynamic web-app surfaces

For acting on behalf of the user, mobile is straightforward:

```text
native prompt -> workflow endpoint -> app-action MCP -> native app state update
```

For source/view mutation, mobile has narrower options:

- render workflow output through existing native components
- use a schema-driven/server-driven UI renderer
- use a WebView for full dynamic Agent App surfaces
- ship multiple native workflows/endpoints for iOS, Android, and web variants

The source-code mutation part is naturally strongest on web. Native can support
similar behavior only within predefined dynamic surfaces or WebViews.

## Open Design Decisions

These should be resolved in an implementation plan:

- exact `kind: "response"` schema
- overlay storage and garbage collection
- session identity for session overlays and per-caller ordering
- persistent patch diff/rollback format
- generated MCP action schema and transport adapters
- how endpoint roles map to Cloud auth or self-hosted auth
- initial replica pool implementation in local runtime and hosted containers
- external integration package shape for Node/Next/Vercel-style apps
- mobile dynamic surface strategy
