# M9 Agent Apps Self-Deployment Plan

This plan extends workflow publication so Arroba-deployed workflows can become
Agent Apps. It covers only Arroba self-deployment through the existing
`local_runtime` and `hosted_container` deployment modes. External web app
middleware, Vercel integration, Git patching, and arbitrary third-party app
hosting are out of scope for this phase.

OpenCode validation is blocked and stays outside this goal's validation and
completion criteria; it will be reintroduced as a separate validation target
after the blocker is resolved.

## Summary

An Agent App deployment is a workflow publication package with static app
assets, workflow-wrapped routes, response effects, overlays, app actions,
persistent patches, endpoint roles, and optional workflow replica pools.

The implementation must build on the existing publication gateway, Cloud
deployment records, Hetzner ingress/runner, Workflow Deployments tab, and canvas
deploy modal. Do not create a second runtime path.

## Package And Protocol

- Bump the local daemon protocol version and update protocol shape/hash tests.
- Add `agent_app` to exported publication packages and set package version to
  `2` when Agent App metadata is present.
- Keep existing publication packages valid as non-Agent-App packages.
- Add export options for:
  - packaged app asset directory
  - wrapped app routes
  - app actions
  - endpoint role/manipulation policy
  - replica pool configuration
  - persistent patch enablement
- Export copies the selected built/static asset directory into package path
  `app/`.
- Store Agent App metadata in `publication.json`:

```json
{
  "agent_app": {
    "enabled": true,
    "assets": {
      "public_dir": "app",
      "index": "index.html"
    },
    "routes": [
      {
        "path": "/add/*",
        "hook_id": "pub-hook",
        "prompt_source": "path_tail",
        "response": "streaming_shell",
        "required_role": "public",
        "manipulation": {
          "level": "state_and_overlay",
          "scope": "session",
          "allowed_paths": ["/generated/**", "/views/**"],
          "protected_paths": ["/auth/**", "/payments/**"],
          "allowed_actions": ["cart.search", "cart.add", "cart.checkout"]
        }
      }
    ],
    "actions": {},
    "replicas": {
      "count": 1,
      "per_caller_ordering": true,
      "max_queue_depth": 100,
      "timeout_ms": 300000
    },
    "persistent_patch": {
      "enabled": false
    }
  }
}
```

## Gateway Runtime

- Current `arroba-workflow-gateway` becomes the Agent App Gateway when
  `agent_app.enabled`.
- Serve normal app asset paths from `app/`.
- Match wrapped routes before static asset fallback.
- For wrapped browser routes, return the existing streaming viewer shell
  immediately, including queued/running status, partial output, traces, and
  final response.
- Interpret final workflow output as generalized response:

```json
{
  "kind": "response",
  "response": {
    "mode": "serve",
    "entry": "/checkout.html"
  },
  "effects": {
    "overlay": [
      {
        "path": "/generated/checkout.html",
        "mime_type": "text/html",
        "content": "<!doctype html>..."
      }
    ],
    "state": {}
  }
}
```

- Support response modes:
  - `serve`
  - `html`
  - `json`
  - `redirect`
  - `status`
- Keep existing `{ "kind": "html", "html": "..." }` final output working by
  normalizing it to generalized response mode `html`.
- Resolve app content in this order:

```text
session or invocation overlay
  then persistent patch
  then packaged app assets
```

## App Actions

- Add action manifest support under `agent_app.actions`.
- Implement generated app-action tools through the existing kernel/tool or MCP
  pattern.
- Expose only actions listed in the matched route's `allowed_actions`.
- Validate action input schemas before execution.
- Call configured internal HTTP action targets reachable by the gateway or
  hosted container.
- Audit action calls in deployment logs.
- For this self-deployment phase, action targets are internal URLs owned by the
  packaged app/runtime. External third-party app action bridges are out of scope.

## Overlays And Persistent Patch

- Invocation overlay is scoped to one workflow invocation.
- Session overlay is keyed by caller/session cookie and is the browser default
  for Agent App routes.
- Persistent patch is disabled by default.
- Persistent patch is allowed only when both package policy and route
  manipulation level allow `persistent_patch`.
- Persistent patches are stored in deployment runtime storage, logged with diff
  metadata, and survive restart only while the deployment runtime volume
  survives.
- Do not implement Git commits, platform redeploys, or external source patching
  in this phase.

## Replica Pool Scheduling

- Add deployment-level replica configuration for Agent Apps.
- Each replica is a separate hidden materialized publication runtime session
  from the same workflow snapshot.
- Add a pool scheduler before kernel invocation:
  - dispatch to an idle replica when available
  - preserve per-caller ordering
  - queue at pool level when all replicas are busy
  - then invoke the existing kernel workflow queue inside the selected replica
- Product-visible queue is the pool queue. Kernel queues remain per-replica
  execution internals.

## Cloud And Web UI

- Preserve existing deployment modes:
  - `hosted_container`
  - `local_runtime`
- Add optional deployment metadata for Agent Apps:
  - enabled flag
  - app route count
  - app asset index
  - replica count
  - persistent patch enabled flag
- Workflow Deployments tab shows:
  - Agent App badge
  - route count/routes
  - replica count
  - public URL
  - status/logs/errors
- Canvas deploy modal gains an Agent App section:
  - enable Agent App
  - app assets path
  - wrapped route path
  - prompt source: `path_tail`
  - manipulation level
  - replica count
  - persistent patch enabled/disabled
- Human HTTP open action embeds the Agent App URL in the central panel, using
  the existing central display behavior for publication URLs.

## Test Plan

- OSS/kernel/server tests:
  - protocol version and shape tests for new export fields and package v2
  - package export with app assets, route manifest, action manifest, role
    policy, replica config
  - gateway serves static app assets from `app/`
  - wrapped route extracts prompt from path tail
  - response modes: `serve`, `html`, `json`, `redirect`, `status`
  - overlay precedence: session/invocation overlay beats persistent patch beats
    base asset
  - protected paths cannot be overlaid or patched
  - app actions expose only allowed route actions
  - persistent patch disabled by default and rejected unless enabled by policy
  - replica scheduler preserves per-caller ordering and dispatches different
    callers to idle replicas

- Cloud/API/web tests:
  - deployment create/upload/start preserves Agent App metadata
  - hosted container runner materializes package v2 and starts gateway
  - local-runtime deployment registers Agent App backend
  - Workflow Deployments tab renders Agent App metadata
  - canvas deploy modal sends Agent App export/deploy options
  - central panel opens Agent App URL

- Live validation drills:
  - hosted-container Agent App with real provider CLIs and development-mounted
    provider/Arroba credentials for available providers. OpenCode is blocked
    and explicitly excluded from this goal's validation criteria.
  - local-runtime ingress Agent App with the same package and available
    providers, excluding blocked OpenCode
  - browser GET prompt path returns streaming shell, traces, partials, and final
    rendered app view
  - app action drill: prompt modifies app state through allowed action
  - overlay drill: generated dashboard asset overrides base app for session only
  - persistent patch drill: admin route changes a permitted asset, survives
    restart, then rolls back
  - replica drill: two callers dispatch to separate replicas; same caller
    preserves ordering
  - screenshots saved under `.artifacts/agent-apps-self-deployment/`
  - cleanup containers, temp packages, runtime dirs, and runner residue after
    drills

## Shopping Agent App End-To-End Drill

After the plan is implemented, run a full browser-visible drill with a simple
shopping web app.

The packaged app must include:

- home page with product list
- cart state
- checkout page
- internal action endpoints for:
  - product search
  - add to cart
  - prepare checkout
  - mark checkout ready
- wrapped route:

```text
GET /add/*
```

Example browser URL:

```text
http://127.0.0.1:<port>/add/1 kg bananas, 2 bottles of 1l Coca-Cola, and a bag of chips
```

Expected flow:

1. Browser loads the wrapped route from the URL bar.
2. Gateway extracts the path tail as the prompt.
3. Gateway returns the streaming shell immediately.
4. Workflow reads the shopping list.
5. Workflow calls allowed app actions to search products and add them to cart.
6. Workflow prepares checkout automatically.
7. Workflow emits an overlay that customizes the checkout page according to the
   shopping list.
8. Final response serves the checkout page from overlay/session state.
9. Browser shows the customized checkout page.
10. Trace pane shows exposed workflow traces, including action/tool use.

Validation requirements:

- Run locally through `arroba serve`.
- Run through `local_runtime` Cloud ingress.
- Run as `hosted_container`.
- Validate URL-bar prompt invocation.
- Validate automatic cart/checkout action effects.
- Validate checkout overlay customization.
- Validate session isolation by opening a second browser session with a
  different shopping list and confirming the first checkout is unchanged.
- Validate screenshots for:
  - streaming shell while queued/running
  - final customized checkout
  - trace pane with action/tool traces
  - Cloud central-panel embedded view
- Save evidence under:

```text
.artifacts/agent-apps-self-deployment/shopping-drill/
```

## Assumptions

- Both existing deployment modes are in scope: `hosted_container` and
  `local_runtime`.
- Base apps are packaged built/static assets; no source build step runs inside
  the deployed container for this phase.
- External web app middleware, Vercel integration, Git patching, and arbitrary
  external deployment environments are out of scope.
- Endpoint roles are stored in package/deployment policy now, but v1 enforcement
  is limited to Arroba-controlled deployment surfaces and route policy checks.
- Product packages must not permanently embed provider credentials, provider
  CLI credential state, or Arroba Cloud account credentials. Development and
  validation drills may use provider CLIs plus mounted or runner-provided
  provider CLI credentials and Arroba credentials so the full hosted-container
  and local-runtime pipelines can be tested with real providers.
- OpenCode validation is currently blocked and is explicitly outside this goal
  and its completion criteria. Agent Apps validation should proceed with the
  available real providers; OpenCode should be reintroduced as a separate
  validation target once the blocker is resolved.
