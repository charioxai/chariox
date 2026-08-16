import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

import { AgentTerminal, type AgentTerminalContext } from "./agent-terminal.js"
import type { TerminalOperationContract, TerminalOperationRegistry } from "./kernel-types-terminal.js"

type ManifestEntry = { variant: string; classification: string }
type ContractFile = {
  contracts: Record<string, { required_targets: string[]; input_schema: Record<string, unknown> | null }>
}

const manifest = JSON.parse(await readFile(new URL("../../../apps/kernel/src/runtime/terminal_operation_registry/parity_manifest.json", import.meta.url), "utf8")) as { requests: ManifestEntry[] }
const contractFile = JSON.parse(await readFile(new URL("../../../apps/kernel/src/runtime/terminal_operation_registry/contracts.json", import.meta.url), "utf8")) as ContractFile

function snakeCase(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase()
}

function operationRegistry(): TerminalOperationRegistry {
  const operations = manifest.requests
    .filter((entry) => entry.classification === "agent_terminal_supported")
    .map((entry): TerminalOperationContract => {
      const contract = contractFile.contracts[entry.variant]
      assert.ok(contract, `missing generated contract for ${entry.variant}`)
      return {
        id: `terminal.${snakeCase(entry.variant)}`,
        description: entry.variant,
        required_targets: contract.required_targets,
        input_schema: contract.input_schema,
        result_kind: "kernel_response",
        mutation: true,
        supported_surfaces: ["agent_terminal"],
        parity_variants: [entry.variant],
        presentation_only: false,
      }
    })
  return { revision: "contract-matrix", operations }
}

function valueForSchema(schema: Record<string, unknown> | null | undefined): unknown {
  if (!schema) return null
  if (schema.const !== undefined) return schema.const
  if (Array.isArray(schema.enum) && schema.enum.length > 0) return schema.enum[0]
  if (Array.isArray(schema.oneOf) && schema.oneOf.length > 0) return valueForSchema(schema.oneOf[0] as Record<string, unknown>)
  switch (schema.type) {
    case "null": return null
    case "boolean": return false
    case "integer": return 1
    case "number": return 1
    case "string": return "matrix-value"
    case "array": return [valueForSchema(schema.items as Record<string, unknown> | undefined)]
    case "object": {
      const properties = schema.properties && typeof schema.properties === "object"
        ? schema.properties as Record<string, Record<string, unknown>>
        : {}
      return Object.fromEntries(Object.entries(properties).map(([key, property]) => [key, valueForSchema(property)]))
    }
    default: return {}
  }
}

function invalidValueForSchema(schema: Record<string, unknown> | null | undefined): unknown {
  return schema?.type === "null" ? {} : { __agent_terminal_invalid: true }
}

function contextFor(targets: string[] = []): AgentTerminalContext {
  return {
    workspace: "/matrix/workspace",
    worktree: "/matrix/workspace",
    workspace_id: "workspace-matrix",
    worktree_id: "worktree-matrix",
    session_id: "session-matrix",
    attachment_id: "attachment-matrix",
    agent_id: "agent-matrix",
    workflow_id: "workflow-matrix",
    targets: Object.fromEntries(targets.map((target) => [target, `target-${target}`])),
  }
}

function targetInputKey(target: string, variant: string): string {
  return target === "agent_id" && variant === "SubmitPrompt" ? "target_agent_id" : target
}

function contextTargetValue(context: AgentTerminalContext, target: string): string | null | undefined {
  if (target === "workspace_id") return context.workspace_id
  if (target === "worktree_id") return context.worktree_id
  if (target === "session_id") return context.session_id
  if (target === "attachment_id") return context.attachment_id
  if (target === "agent_id" || target === "target_agent_id") return context.agent_id
  if (target === "workflow_id" || target === "workflow_ref") return context.workflow_id
  return context.targets?.[target]
}

function clearContextTarget(context: AgentTerminalContext, target: string): void {
  if (target === "workspace_id") context.workspace_id = null
  else if (target === "worktree_id") context.worktree_id = null
  else if (target === "session_id") context.session_id = null
  else if (target === "attachment_id") context.attachment_id = null
  else if (target === "agent_id" || target === "target_agent_id") context.agent_id = null
  else if (target === "workflow_id" || target === "workflow_ref") context.workflow_id = null
  if (context.targets) delete context.targets[target]
}

function fakeClient(registry: TerminalOperationRegistry) {
  const requests: Record<string, unknown>[] = []
  const client = {
    requests,
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetTerminalOperationRegistry" in request) return { TerminalOperationRegistry: { registry } }
      if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-matrix" } } }
      if ("GetSessionState" in request) return { SessionState: { session: { agent_activity: {} } } }
      return {}
    },
  }
  return client
}

test("agent terminal executor covers every generated structured request contract", async () => {
  const registry = operationRegistry()
  const client = fakeClient(registry)
  const terminal = new AgentTerminal(client, "contract-matrix-client")
  for (const operation of registry.operations) {
    const variant = operation.parity_variants?.[0]
    assert.ok(variant, `${operation.id} has no native request variant`)
    const context = contextFor(operation.required_targets)
    const input = operation.input_schema?.type === "null" ? undefined : valueForSchema(operation.input_schema)
    const before = client.requests.length
    await terminal.executeOperation(operation.id, input, context)
    assert.ok(client.requests.length > before, `${operation.id} did not dispatch a request`)
    const request = client.requests.at(-1) ?? {}
    assert.ok(Object.prototype.hasOwnProperty.call(request, variant), `${operation.id} dispatched the wrong variant`)
    const payload = request[variant]
    if (payload && typeof payload === "object" && !Array.isArray(payload)) {
      const object = payload as Record<string, unknown>
      for (const target of operation.required_targets ?? []) {
        if (target === "attachment_id") continue
        const key = targetInputKey(target, variant)
        const expected = contextTargetValue(context, target)
        if (expected !== undefined) assert.equal(object[key], expected, `${operation.id} did not make ${target} authoritative`)
      }
      if (variant === "SubmitPrompts" && Array.isArray(object.prompts)) {
        for (const prompt of object.prompts as Record<string, unknown>[]) {
          assert.equal(prompt.session_id, context.session_id)
          assert.equal(prompt.attachment_id, context.attachment_id)
          assert.equal(prompt.target_agent_id, context.agent_id)
          assert.equal(prompt.prompt_source, "agent_terminal")
        }
      }
    }
  }
  await terminal.close()
})

test("agent terminal rejects a missing explicit target for every targeted structured contract", async () => {
  const registry = operationRegistry()
  for (const operation of registry.operations.filter((candidate) => (candidate.required_targets ?? []).some((target) => !["attachment_id", "workspace_id", "worktree_id"].includes(target)))) {
    const missingTarget = (operation.required_targets ?? []).find((target) => !["attachment_id", "workspace_id", "worktree_id"].includes(target))!
    const context = contextFor(operation.required_targets?.filter((target) => target !== missingTarget))
    clearContextTarget(context, missingTarget)
    const client = fakeClient(registry)
    const terminal = new AgentTerminal(client, "missing-target-matrix-client")
    await assert.rejects(
      () => terminal.executeOperation(operation.id, valueForSchema(operation.input_schema), context),
      new RegExp(`requires explicit ${missingTarget}`),
    )
    assert.equal(client.requests.filter((request) => Object.prototype.hasOwnProperty.call(request, operation.parity_variants?.[0] ?? "")).length, 0, `${operation.id} dispatched after missing-target validation`)
    await terminal.close()
  }
})

test("agent terminal rejects an invalid structured payload for every generated contract", async () => {
  const registry = operationRegistry()
  for (const operation of registry.operations) {
    const variant = operation.parity_variants?.[0]
    assert.ok(variant, `${operation.id} has no native request variant`)
    const client = fakeClient(registry)
    const terminal = new AgentTerminal(client, "invalid-input-matrix-client")
    await terminal.getRegistry()
    const before = client.requests.length
    await assert.rejects(
      () => terminal.executeOperation(operation.id, invalidValueForSchema(operation.input_schema), contextFor(operation.required_targets)),
      /invalid input/,
      operation.id,
    )
    assert.equal(
      client.requests.slice(before).some((request) => Object.prototype.hasOwnProperty.call(request, variant)),
      false,
      `${operation.id} dispatched after invalid-input validation`,
    )
    await terminal.close()
  }
})
