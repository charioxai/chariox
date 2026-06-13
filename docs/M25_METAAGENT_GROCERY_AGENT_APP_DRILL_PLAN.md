# M25 Metaagent Grocery Agent App Drill Plan

## Objective

Create an end-to-end drill that proves a session metaagent can supervise a
small team of regular agents through a realistic product workflow:

1. Build a simple online grocery store web app.
2. Convert it into an agent app driven by a grocery-list prompt.
3. Publish/export it as a container.
4. Deploy it through the headlessnet and make it reachable from the Scalingo
   staging development path.

The drill should exercise real Arroba runtime paths:

- Browser Cloud UI.
- Hosted staging relay.
- Local kernel connected to the hosted relay.
- Kernel-owned session, workflow, agent, runtime tool, event, and publication
  paths.
- Metaagent supervision from the terminal and Metaagents side panel.

Cloud must remain bootstrap/control plane only. Runtime terminal traffic and
metaagent actions must continue to route through kernel/relay surfaces.

## Proposed Location

Implement the drill in `arroba-cloud`:

```text
scripts/staging-metaagent-grocery-agent-app-drill.mjs
```

Store run artifacts under:

```text
.artifacts/metaagent-grocery-agent-app/<run-id>/
```

The drill should write:

- `manifest.json`
- browser screenshots
- kernel/client logs
- provider process diagnostics
- exported package/container metadata
- deployed URL metadata

## Preflight

Before starting the product flow, assert:

- Scalingo staging Cloud URL is reachable.
- Dev auth credentials are available.
- Hosted relay URL is `wss://` for staging.
- A local kernel can connect to the hosted relay.
- The staging dashboard shows the local kernel relay target as fresh by
  heartbeat.
- Provider catalog is available.
- The workspace is an isolated temporary project directory.
- Artifact directory is empty or newly created for the run.

Screenshots:

- Staging login or dev-auth landing state.
- Waiting room with connected staging relay target.
- Create-session controls with metaagent enabled.

## Phase 1: Create A Metaagent Session

Use the browser product flow to create a new session with:

- One session metaagent.
- Local temporary workspace.
- Permission mode suitable for the drill.
- No slice placement for the metaagent.

Then launch the metaagent provider run through normal runtime paths.

Acceptance:

- Session has exactly one metaagent.
- Duplicate metaagent creation is rejected through product UI.
- Metaagent appears in the terminal footer and Metaagents side panel.
- Metaagent runtime tools are available only to the metaagent provider run.
- Workflow node candidate lists exclude the metaagent.

Screenshots:

- Created terminal session.
- Metaagent-focused terminal pane.
- Metaagents side panel overview.
- Duplicate metaagent spawn rejection.

## Phase 2: Prompt The Metaagent To Build The Grocery Store

Submit one user-visible prompt to the metaagent:

```text
Create and supervise a workflow that builds a small online grocery store web
app in this workspace. Create a project manager agent and one or more developer
agents. Use the workflow to coordinate implementation. The app must be local
only, with no external services. It must support registration, login, product
sections, product cards with name, price, stock, and product details, a basket,
checkout, and a fake purchase confirmation.
```

The drill should not hand-create the workflow unless diagnosing a failure. The
metaagent should choose the task force shape, but the drill should validate that
the result includes at least:

- A project-manager-like regular agent.
- One or more developer-like regular agents.
- A workflow containing those regular agents.
- No metaagent as a workflow node.

Expected app scope:

- Registration and login using local browser storage or a simple local data
  file.
- Product categories such as produce, bakery, pantry, dairy, and frozen.
- Product cards with visible price and stock.
- Product detail or expanded product state.
- Basket/cart with quantity controls.
- Checkout screen.
- Fake purchase confirmation.
- No payment provider.
- No external database.
- No third-party hosted service.

Acceptance:

- App source files exist in the workspace.
- App can be started locally by the drill.
- Browser can register a user.
- Browser can log in as that user.
- Browser can view categories/products.
- Browser can add multiple products to the basket.
- Basket totals are shown.
- Checkout can be completed.
- Fake purchase confirmation is shown.

Screenshots:

- Workflow canvas with PM/developer agents.
- Metaagent side panel showing owned regular agents.
- Grocery app catalog.
- Product detail or expanded product state.
- Basket.
- Checkout.
- Purchase confirmation.

## Phase 3: Convert The Grocery Store Into An Agent App

Submit a second prompt to the metaagent:

```text
Convert the grocery store into an agent app. The app should accept a grocery
list prompt from the browser URL/input surface, such as "milk, eggs, bread,
apples". It should parse the list, add matching available products to the
basket, choose simple substitutions when stock is unavailable, and take the user
to checkout. Keep the app local and fake. Use the same workflow or create a new
workflow if that is cleaner.
```

The metaagent may reuse the original team or create a second workflow. The drill
should accept either, but the workflow state must remain visible and auditable.

Acceptance:

- Agent app route/package metadata exists.
- User can enter a grocery-list prompt from the browser URL/input surface.
- Prompted items appear in the basket.
- Out-of-stock behavior is deterministic and visible.
- Checkout is reached from the prompt-driven flow.
- Fake purchase can complete.

Screenshots:

- Agent app prompt/input state.
- Parsed grocery-list result.
- Auto-populated basket.
- Checkout reached from prompt.
- Fake purchase confirmation.

## Phase 4: Publish, Export, And Deploy

Prompt the metaagent to publish/export:

```text
Publish the grocery agent app, export it as a container, and deploy it to the
headlessnet so it is reachable through the Scalingo staging development path.
After deployment, verify the grocery-list prompt flow on the staged URL.
```

The drill should verify publication and deployment through existing product and
kernel paths. It should not invent a Cloud-only publication path.

Acceptance:

- Workflow publication exists.
- Exported package exists.
- Container export/build metadata exists.
- Deployment to headlessnet succeeds.
- Scalingo staging development path resolves to the deployed app.
- Staged URL loads the grocery agent app.
- Staged URL supports the grocery-list-to-checkout flow.

Screenshots:

- Publication or deployment panel.
- Export/container artifact view.
- Scalingo staging development URL loading.
- Staged grocery-list prompt.
- Staged checkout/confirmation.

## Metaagent Capability Checks

The drill should explicitly check these metaagent capabilities while the product
flow runs:

- `arroba.meta.search_commands` or command palette search works.
- Metaagent can inspect session overview.
- Metaagent can inspect owned regular agents.
- Metaagent can prompt owned regular agents.
- Metaagent event inbox receives worker turn-completion events.
- Metaagent can subscribe to workflow events.
- Subscribed workflow output events appear in the Metaagents side panel.
- Pending runtime interactions are visible and resolvable through the web UI
  when they occur.
- Duplicate metaagent creation is rejected.
- Metaagent is not available as a workflow node.
- Kernel rejection messages surface in product UI.

## Manifest

The final manifest should include:

```json
{
  "ok": true,
  "runId": "staging-metaagent-grocery-agent-app-...",
  "sessionId": "...",
  "metaagentId": "...",
  "regularAgentIds": ["..."],
  "workflowIds": ["..."],
  "localAppUrl": "...",
  "publishedUrl": "...",
  "containerArtifact": "...",
  "headlessnetDeployment": {
    "id": "...",
    "status": "ready"
  },
  "checkpoints": {
    "metaagentSessionCreated": true,
    "workflowCreatedByMetaagent": true,
    "groceryAppBuilt": true,
    "registrationLoginPassed": true,
    "basketCheckoutPassed": true,
    "agentAppPromptPassed": true,
    "publicationCreated": true,
    "containerExported": true,
    "headlessnetDeployed": true,
    "stagingPathVerified": true
  },
  "screenshots": {}
}
```

On failure, include:

- current URL
- focused session/agent
- workflow summaries
- metaagent event counts
- provider process diagnostics
- last visible product UI status
- all screenshots captured before failure

## Timeouts And Failure Boundaries

Use explicit time boxes per phase:

- Bootstrap: 2 minutes.
- Metaagent session creation: 2 minutes.
- Grocery app workflow creation: 5 minutes.
- Grocery app implementation: 20 minutes.
- Local app validation: 5 minutes.
- Agent app conversion: 15 minutes.
- Publication/export: 10 minutes.
- Headlessnet deployment and staging verification: 10 minutes.

If a phase times out, fail the drill with diagnostics. Do not silently repair the
workflow from the drill unless the failure is explicitly marked as a drill
diagnostic mode.

## Cleanup

After the run:

- Stop local app servers.
- Stop provider processes launched by the drill.
- Close browser context.
- Remove temporary workspaces unless `KEEP_ARTIFACTS=1`.
- Keep screenshots, logs, manifest, and exported package metadata.
- Do not delete deployment artifacts that are needed to inspect a successful
  staged deployment.

## Release Gate

Treat the drill as passing only when:

- The metaagent creates or supervises the task force workflow.
- The grocery app works locally.
- The agent app grocery-list prompt works locally.
- Publication/export succeeds.
- The container-backed deployment is reachable through the Scalingo staging
  development path.
- The staged deployment supports the grocery-list-to-checkout flow.
- The manifest includes screenshots and checkpoint evidence for every phase.
