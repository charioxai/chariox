import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const contracts = JSON.parse(await readFile(new URL("../apps/kernel/src/runtime/terminal_operation_registry/contracts.json", import.meta.url), "utf8")).contracts
const manifest = JSON.parse(await readFile(new URL("../apps/kernel/src/runtime/terminal_operation_registry/parity_manifest.json", import.meta.url), "utf8"))

test("generated contracts preserve tagged nested request shapes", () => {
  const install = contracts.InstallMcpServer.input_schema.properties.config
  const transport = install.properties.transport
  assert.equal(install.additionalProperties, false)
  assert.equal(transport.oneOf.length, 2)
  assert.deepEqual(transport.oneOf[0].properties.type.const, "stdio")
  assert.equal(transport.oneOf[0].properties.command.type, "string")
  assert.equal(transport.oneOf[1].properties.url.type, "string")
  assert.equal(install.properties.startup_timeout_sec.oneOf[0].type, "integer")
  assert.equal(install.properties.startup_timeout_sec.oneOf[1].type, "null")

  const workflowSource = contracts.AddWorkflowRegistryEntry.input_schema.properties.source
  assert.equal(workflowSource.oneOf.length, 2)
  assert.equal(workflowSource.oneOf[0].properties.kind.const, "single_file")
  assert.equal(workflowSource.oneOf[0].properties.source.type, "string")
  assert.equal(workflowSource.oneOf[1].properties.kind.const, "source_directory")
  assert.equal(workflowSource.oneOf[1].properties.files.items.additionalProperties, false)

  const workflowPatchAlias = contracts.ApplyWorkflowDesignOp.input_schema.properties.op.oneOf[1].properties.patch.properties.alias
  assert.equal(workflowPatchAlias.oneOf[0].oneOf[0].type, "string")
  assert.equal(workflowPatchAlias.oneOf[0].oneOf[1].type, "null")
  assert.equal(workflowPatchAlias.oneOf[1].type, "null")
})

test("generated contracts preserve externally tagged enum payloads and aliases", () => {
  const substituteAction = contracts.UpdateAgentSubstitutes.input_schema.properties.action
  assert.equal(substituteAction.oneOf[0].properties.Add.properties.provider.type, "string")
  assert.equal(substituteAction.oneOf[1].properties.Remove.properties.index.type, "integer")
  assert.equal(substituteAction.oneOf[2].properties.Clear.type, "object")

  const utilityInput = contracts.RunAgentUtility.input_schema.properties.input
  assert.equal(utilityInput.oneOf[0].properties.WorkspaceCommitMessage.properties.workspace_id.type, "string")
  assert.equal(utilityInput.oneOf[1].properties.SemanticRecallSearch.properties.query.type, "string")

  const watchdogPolicy = contracts.CreateWorkflowWatchdog.input_schema.properties.policy
  assert.deepEqual(watchdogPolicy.enum, ["skip", "queue"])
})

test("every supported request has a closed, target-consistent contract", () => {
  const supported = manifest.requests.filter((entry) => entry.classification === "agent_terminal_supported")
  assert.equal(Object.keys(contracts).length, supported.length)
  for (const entry of supported) {
    const contract = contracts[entry.variant]
    assert.ok(contract?.input_schema, `${entry.variant} has no input schema`)
    const schema = contract.input_schema
    assert.ok(schema.type === "null" || (schema.type === "object" && schema.additionalProperties === false), `${entry.variant} is not a closed top-level schema`)
    if (schema.type === "object") {
      for (const target of contract.required_targets ?? []) {
        assert.ok(Object.prototype.hasOwnProperty.call(schema.properties ?? {}, target), `${entry.variant} target ${target} is absent from properties`)
      }
    }
  }
})

test("plaintext credential entry is not exposed to agent terminals", () => {
  assert.equal(manifest.requests.find((entry) => entry.variant === "SetCredentialSecret")?.classification, "presentation_only")
  assert.equal(contracts.SetCredentialSecret, undefined)
})
