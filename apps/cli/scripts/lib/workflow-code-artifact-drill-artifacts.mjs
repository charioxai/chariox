import { applyWorkflowCodeArtifactRequest, createWorkflowCodeArtifactRequest, invokeWorkflowEndpointRequest, launchProviderRunRequest, spawnAgentRequest } from '@chariox/kernel-client'
import { assert, existingAgentRebindings, existingAgentWorkflowCodeSource, outputSchemaWorkflowCodeSource, unwrap } from './workflow-code-artifact-drill-runtime.mjs'
import { validateApplyResult, validateSessionProjection } from './workflow-code-artifact-drill-topology.mjs'
import { waitForCompletedWorkflowRun, waitForProviderRunReady } from './workflow-code-artifact-drill-waits.mjs'

export async function applyExistingAgentArtifact(client, session, nodePath, workspace) {
  const existingAgent = unwrap(
    await client.send(spawnAgentRequest(
      session.id,
      'dev-stub',
      'artifact-existing-worker',
      'default',
      workspace,
      'low',
    )),
    'AgentSpawned',
  ).agent
  const artifactName = `existing-agent-artifact-${Date.now()}`
  const source = existingAgentWorkflowCodeSource(existingAgent.id)
  const created = unwrap(
    await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
    'WorkflowCodeArtifactCreated',
  ).artifact
  assert(created?.metadata?.validation?.ok, 'existing-agent artifact should validate', created?.metadata?.validation)

  const expected = {
    nodes: 2,
    agents: 2,
    edges: 1,
    endpoints: 1,
    requiredSchemas: ['existing_handoff', 'final_output'],
  }
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName, existingAgentRebindings())),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, 'existing-agent artifact apply', expected)
  validateSessionProjection(appliedResponse.session, apply, 'existing-agent artifact apply', expected)
  assert(
    apply.agent_ids?.existing_worker === existingAgent.id,
    'existing-agent artifact should preserve the pre-existing agent id for its node',
    { apply, existingAgent },
  )
  const generatedAgentId = apply.agent_ids?.generated_finisher
  assert(
    generatedAgentId && generatedAgentId !== existingAgent.id,
    'existing-agent artifact should create a distinct generated node agent',
    { apply, existingAgent },
  )
  return {
    artifactName,
    workflowId: apply.workflow_id,
    existingAgentId: existingAgent.id,
    generatedAgentId,
  }
}

export async function applyOutputSchemaArtifact(client, session, nodePath, timeoutMs) {
  const artifactName = `output-schema-artifact-${Date.now()}`
  const source = outputSchemaWorkflowCodeSource()
  const created = unwrap(
    await client.send(createWorkflowCodeArtifactRequest(session.id, artifactName, nodePath, source)),
    'WorkflowCodeArtifactCreated',
  ).artifact
  assert(created?.metadata?.validation?.ok, 'output-schema artifact should validate', created?.metadata?.validation)

  const expected = {
    nodes: 1,
    agents: 1,
    edges: 0,
    endpoints: 1,
    requiredSchemas: ['value_output', 'progress_output'],
  }
  const appliedResponse = unwrap(
    await client.send(applyWorkflowCodeArtifactRequest(session.id, artifactName)),
    'WorkflowCodeApplied',
  )
  const apply = validateApplyResult(appliedResponse.result, 'output-schema artifact apply', expected)
  const workflow = (appliedResponse.session?.workflows ?? []).find((entry) => entry.id === apply.workflow_id)
  assert(workflow, 'output-schema artifact workflow should appear in session projection', { workflowId: apply.workflow_id })
  assert(
    workflow.run_output_schema_ref === apply.schema_refs.value_output,
    'output-schema artifact should assign workflow final output schema',
    { workflow, schemaRefs: apply.schema_refs },
  )
  const nodeId = apply.node_ids?.worker
  const agentId = apply.agent_ids?.worker
  const node = (workflow.nodes ?? []).find((entry) => entry.id === nodeId)
  assert(node?.intermediate_output_schema_ref === apply.schema_refs.progress_output, 'output-schema artifact should assign node intermediate schema', {
    node,
    schemaRefs: apply.schema_refs,
  })
  assert(agentId, 'output-schema artifact should resolve worker agent id', apply)

  const launchResponse = unwrap(
    await client.send(launchProviderRunRequest(
      session.id,
      'dev-stub',
      'default',
      'workflow-intermediate-node',
      'low',
      agentId,
    )),
    'ProviderRunLaunchAccepted',
  )
  assert(launchResponse?.provider_run?.id, 'output-schema artifact should launch generated worker provider run', launchResponse)
  await waitForProviderRunReady(client, launchResponse.provider_run.id, timeoutMs)

  const endpointId = apply.endpoint_ids?.entry
  assert(endpointId, 'output-schema artifact should resolve entry endpoint id', apply)
  const invokeResponse = await client.send(invokeWorkflowEndpointRequest(
    session.id,
    apply.workflow_id,
    endpointId,
    'Run the workflow-code output schema drill.',
  ))
  const workflowRun = unwrap(invokeResponse, 'WorkflowRunInvoked')?.workflow_run
  assert(workflowRun?.id, 'output-schema artifact should start a workflow run', invokeResponse)
  const completed = await waitForCompletedWorkflowRun(client, session.id, workflowRun.id, timeoutMs)
  assert(completed.status === 'Completed', 'output-schema artifact workflow run should complete', completed)
  assert(completed.final_output?.message === JSON.stringify({ value: 1842 }), 'output-schema artifact final output mismatch', completed)
  assert(completed.final_output_valid === true, 'output-schema artifact final output should validate', completed)
  const intermediate = (completed.intermediate_outputs ?? []).find(
    (entry) => entry.output?.message === JSON.stringify({ value: 1841 }),
  )
  assert(intermediate, 'output-schema artifact should record the intermediate output', completed)
  assert(intermediate.valid === true, 'output-schema artifact intermediate output should validate', intermediate)

  return {
    artifactName,
    workflowId: apply.workflow_id,
    workflowRunId: workflowRun.id,
    finalOutput: completed.final_output?.message,
    intermediateOutputs: completed.intermediate_outputs?.map((entry) => entry.output?.message) ?? [],
  }
}
