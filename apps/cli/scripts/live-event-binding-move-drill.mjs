#!/usr/bin/env node
// Focused drill for moving a workflow event binding to an App (protocol 360).
// Against a live kernel with the dummy event generator connected, an agent in
// the session, and an installed App that declares an incoming and an outgoing
// event:
//   node scripts/live-event-binding-move-drill.mjs --session ID --agent ID \
//     --installation ID --incoming NAME --outgoing NAME --connection ID \
//     [--scope default] [--kernel-url ws://127.0.0.1:44240/kernel] [--evidence FILE]
// It makes a one-node workflow with an event publication and a dummy binding,
// checks that a refused move (an undeclared outgoing event) leaves the binding
// active with no route or automation, moves it for real, and removes what it made.
import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { writeFile } from "node:fs/promises"

import { LocalIpcClient } from "../dist/ipc.js"

const options = parseArgs(process.argv.slice(2))
const client = new LocalIpcClient(options.kernelUrl)
const tag = randomUUID().slice(0, 8)
const route = `move-${tag}`
const automation = `move-${tag}`
const evidence = { started_at: new Date().toISOString(), steps: [] }
const created = { route: false, automation: false, publication: null }
const app = { installation_id: options.installation }

try {
  const detail = (await client.send({ GetEventGeneratorDetail: { generator_id: "dev.chariox.dummy" } })).EventGeneratorDetail
  const generator = detail.detail ?? detail
  const { workflow, endpoint } = (await client.send({ CreateAgentWorkflow: {
    session_id: options.session, agent_id: options.agent, reason: "trigger", surface: "cli", alias: `move-${tag}`,
  } })).AgentWorkflowCreated
  const publication = (await client.send({ CreateWorkflowPublication: {
    session_id: options.session, workflow_ref: workflow.id, endpoint_ref: endpoint.id, expected_workflow_revision: null,
    operation_key: `publish-move-${tag}`, queue_ref: "default", alias: `move-${tag}`, kind: "event_based", route: null,
    methods: [], transport: null, parser: null, input_schema: null, trace_exposure: null, mode: null,
    sync_timeout_ms: null, poll_ms: null,
  } })).WorkflowPublicationCreated.publication
  created.publication = publication.id
  const binding = (await client.send({ CreateWorkflowEventBinding: {
    session_id: options.session, publication_ref: publication.id, generator_id: "dev.chariox.dummy",
    generator_version: generator.version, manifest_digest: generator.manifest_digest, connection_id: options.connection,
    connection_scope: options.scope, event_type: "dummy.test", event_type_version: 1, filter: { channel: `move-${tag}` },
    environment_id: null, queue_ref: "default", action_ids: [],
  } })).WorkflowEventBindingCreated.binding
  evidence.steps.push({ step: "old-path binding", binding: binding.id, publication: publication.id })

  const move = (outgoing) => client.send({ MoveEventBindingToApp: {
    session_id: options.session, binding_id: binding.id, installation_id: options.installation, route_id: route,
    event_name: options.incoming, automation: { automation_id: automation, event_name: outgoing },
  } })
  const status = async () => (await client.send({ ListWorkflowEventBindings: { session_id: options.session, publication_ref: null } }))
    .WorkflowEventBindingsListed.bindings.find((row) => row.id === binding.id)?.status
  const routes = async () => (await client.send({ ListAppInboxRoutes: app })).AppInboxRoutes.routes.map((row) => row.route_id)
  const automations = async () => (await client.send({ ListAppAutomations: app })).AppAutomations.automations
    .map((row) => row.automation_id)

  const refused = await move(`undeclared-${tag}`)
  assert.ok(refused.AppRequestFailed, `a move with an undeclared outgoing event must be refused: ${JSON.stringify(refused)}`)
  assert.equal(await status(), "active", "a refused move reactivates the binding")
  assert.ok(!(await routes()).includes(route), "a refused move leaves no route")
  assert.ok(!(await automations()).includes(automation), "a refused move leaves no automation")
  evidence.steps.push({ step: "refused move undone", answer: refused.AppRequestFailed })

  const moved = (await move(options.outgoing)).EventBindingMovedToApp
  assert.ok(moved, "the move with a declared outgoing event succeeds")
  created.route = created.automation = true
  assert.equal(await status(), "paused", "the moved binding is paused")
  assert.equal(moved.route.connection?.connection_id, options.connection)
  assert.equal(moved.automation?.publication_id, publication.id)
  evidence.steps.push({ step: "moved", route: moved.route.route_id, automation: moved.automation.automation_id })
  console.log("EVENT_BINDING_MOVE_DRILL_PASS")
} finally {
  if (created.route) await client.send({ RemoveAppInboxRoute: { ...app, route_id: route } }).catch(() => {})
  if (created.automation) {
    const current = (await client.send({ ListAppAutomations: app }).catch(() => null))?.AppAutomations?.automations
      .find((row) => row.automation_id === automation)
    if (current) {
      await client.send({ DisableAppAutomation: { ...app, automation_id: automation, expected_revision: current.revision } })
        .catch(() => {})
    }
  }
  if (created.publication) {
    await client.send({ DisableWorkflowPublication: { session_id: options.session, publication_ref: created.publication } })
      .catch(() => {})
  }
  if (options.evidence) await writeFile(options.evidence, `${JSON.stringify(evidence, null, 2)}\n`)
  client.close?.()
}

function parseArgs(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 2) {
    assert.match(argv[index] ?? "", /^--/, `unexpected argument ${argv[index]}`)
    values[argv[index].slice(2)] = argv[index + 1]
  }
  for (const name of ["session", "agent", "installation", "incoming", "outgoing", "connection"]) {
    assert.ok(values[name], `--${name} is required`)
  }
  return {
    session: values.session,
    agent: values.agent,
    installation: values.installation,
    incoming: values.incoming,
    outgoing: values.outgoing,
    connection: values.connection,
    scope: values.scope ?? "default",
    kernelUrl: values["kernel-url"] ?? "ws://127.0.0.1:44240/kernel",
    evidence: values.evidence,
  }
}
