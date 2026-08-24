import assert from 'node:assert/strict'
import test from 'node:test'

import {
  assertActivationBlockedBeforeCredentialSetup,
  assertDeploymentContractMatchesRequirements,
  assertExtensionRequirementsExact,
  assertHostedRejectsLocalOnly,
  assertMissingDefinitionBlockedPublication,
  assertUnrelatedExtensionAbsent,
  drillExtensionCleanupSteps,
  drillExtensionRunNames,
  expectedMcpRequirement,
  localOnlyStdioMcpConfig,
  portableHttpsMcpConfig,
} from './live-cloud-publication-deployment-drill-extensions.mjs'

const DIGEST = 'sha256:' + 'a'.repeat(64)

function portableRequirement(overrides = {}) {
  return {
    id: 'mcp:drill-portable',
    kind: 'mcp',
    name: 'drill-portable',
    version: DIGEST,
    content_digest: DIGEST,
    launch_definition: { kind: 'streamable_http', url: 'https://mcp.example.com/rpc' },
    credential_slots: [{
      slot_id: 'integration:abc',
      kind: 'integration',
      integration: 'drill-portable',
      extension_id: 'mcp:drill-portable',
      role: 'bearer',
      label: 'drill-portable: OAuth or bearer token',
      authentication_method: 'oauth_or_api_key',
      required: true,
      agent_ids: ['agent-1'],
    }],
    network_destinations: [{
      id: 'extension:dst',
      host: { kind: 'exact_dns', value: 'mcp.example.com' },
      ports: [443],
      protocols: ['tls'],
      credential_slot_ids: ['integration:abc'],
    }],
    uses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
    readiness_test: { kind: 'mcp_initialize' },
    portability: { classification: 'portable' },
    ...overrides,
  }
}

function requirementsFixture() {
  const extension = portableRequirement()
  return {
    schema_version: 2,
    extensions: [extension],
    credential_slots: extension.credential_slots,
    network_destinations: extension.network_destinations,
  }
}

test('extension fixtures distinguish portable https and local-only stdio definitions', () => {
  const portable = portableHttpsMcpConfig('drill-mcp', 'https://mcp.example.com/rpc', { bearerTokenCredential: 'cred-1' })
  assert.equal(portable.transport.type, 'streamable_http')
  assert.equal(portable.transport.bearer_token_credential, 'cred-1')

  const localOnly = localOnlyStdioMcpConfig('drill-unrelated')
  assert.equal(localOnly.transport.type, 'stdio')
})

test('exactness assertion accepts exactly the granted set with digest, usage, slots, and network policy', () => {
  const requirements = requirementsFixture()
  assert.doesNotThrow(() => assertExtensionRequirementsExact(requirements, [
    expectedMcpRequirement({
      name: 'drill-portable',
      agentId: 'agent-1',
      nodeIds: ['node-1'],
      credentialSlotCount: 1,
      networkHost: 'mcp.example.com',
      expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
    }),
  ]))
})

test('exactness assertion rejects an unrelated installed-but-ungranted extension in the package', () => {
  const requirements = requirementsFixture()
  requirements.extensions.push(portableRequirement({
    id: 'mcp:drill-unrelated',
    name: 'drill-unrelated',
    launch_definition: null,
    credential_slots: [],
    network_destinations: [],
    portability: { classification: 'local_only', reason: 'stdio', recommendation: 'Use connected ingress.' },
  }))
  assert.throws(
    () => assertExtensionRequirementsExact(requirements, [expectedMcpRequirement({
      name: 'drill-portable',
      agentId: 'agent-1',
      nodeIds: ['node-1'],
      expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
    })]),
    /exactly the workflow-installed-and-granted set/,
  )
  assert.throws(
    () => assertUnrelatedExtensionAbsent(requirements, ['drill-unrelated']),
    /installed-but-ungranted extension drill-unrelated/,
  )
})

test('exactness assertion rejects digest drift, wrong usage mapping, and missing network policy', () => {
  const drifted = requirementsFixture()
  drifted.extensions[0].version = 'sha256:' + 'b'.repeat(64)
  assert.throws(() => assertExtensionRequirementsExact(drifted, [expectedMcpRequirement({
    name: 'drill-portable',
    agentId: 'agent-1',
    nodeIds: ['node-1'],
    expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
  })]), /version must equal content_digest/)

  const remapped = requirementsFixture()
  remapped.extensions[0].uses = [{ agent_id: 'agent-1', node_ids: ['node-other'] }]
  assert.throws(() => assertExtensionRequirementsExact(remapped, [expectedMcpRequirement({
    name: 'drill-portable',
    agentId: 'agent-1',
    nodeIds: ['node-1'],
    expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
  })]), /exact node\/agent grant usage/)

  const openNetwork = requirementsFixture()
  delete openNetwork.extensions[0].network_destinations
  openNetwork.network_destinations = []
  assert.throws(() => assertExtensionRequirementsExact(openNetwork, [expectedMcpRequirement({
    name: 'drill-portable',
    agentId: 'agent-1',
    nodeIds: ['node-1'],
    networkHost: 'mcp.example.com',
    credentialSlotCount: 1,
    expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
  })]), /exact_dns destination/)
})

test('local-only packaged extensions keep no launch definition and carry connected-ingress guidance', () => {
  const requirements = {
    schema_version: 2,
    extensions: [portableRequirement({
      id: 'mcp:drill-local',
      name: 'drill-local',
      launch_definition: null,
      credential_slots: [],
      network_destinations: [],
      portability: { classification: 'local_only', reason: 'stdio MCP', recommendation: 'Use connected ingress or replace this extension with a portable package.' },
    })],
    credential_slots: [],
    network_destinations: [],
  }
  assert.doesNotThrow(() => assertExtensionRequirementsExact(requirements, [expectedMcpRequirement({
    name: 'drill-local',
    agentId: 'agent-1',
    nodeIds: ['node-1'],
    classification: 'local_only',
    expectedUses: [{ agent_id: 'agent-1', node_ids: ['node-1'] }],
  })]))
})

test('deployment contract capabilities must deep-equal the immutable requirements', () => {
  const requirements = requirementsFixture()
  const contract = {
    credential_slots: [{
      slot_id: 'integration:abc',
      kind: 'integration',
      integration: 'drill-portable',
      extension_id: 'mcp:drill-portable',
      authentication_method: 'oauth_or_api_key',
      required: true,
    }],
    capabilities: {
      extensions: requirements.extensions,
      network: { policy_version: 1, default_action: 'deny', destinations: requirements.network_destinations },
    },
  }
  assert.doesNotThrow(() => assertDeploymentContractMatchesRequirements(contract, requirements))
  assert.throws(() => assertDeploymentContractMatchesRequirements(
    { capabilities: { extensions: [], network: { destinations: [] } }, credential_slots: [] },
    requirements,
  ), /deep-equal requirements extensions/)
})

test('negative-path matchers pin the exact rejection semantics', () => {
  assert.doesNotThrow(() => assertMissingDefinitionBlockedPublication(
    'extension `ghost-mcp` is granted to a workflow agent but has no installed mcp definition',
  ))
  assert.throws(() => assertMissingDefinitionBlockedPublication('export ok'), /must fail when a granted extension/)

  assert.doesNotThrow(() => assertHostedRejectsLocalOnly(
    'Hosted deployment cannot run local-only extensions. mcp:x: stdio. Use connected ingress or publish a portable replacement.',
  ))
  assert.throws(() => assertHostedRejectsLocalOnly('Hosted deployment cannot run local-only extensions.'), /connected ingress/)

  assert.doesNotThrow(() => assertActivationBlockedBeforeCredentialSetup(
    'Required deployment credentials are not ready: integration:abc (bearer)',
  ))
  assert.throws(() => assertActivationBlockedBeforeCredentialSetup('started'), /activation must block/)
})

test('drill extension run names are registry-safe and unique per run stamp', () => {
  const first = drillExtensionRunNames('abc-123')
  const second = drillExtensionRunNames('def-456')

  for (const name of Object.values(first)) {
    assert.match(name, /^drill-[a-z0-9-]+$/)
    assert.ok(!name.includes('drill-drill'))
  }
  for (const key of Object.keys(first)) {
    assert.notEqual(first[key], second[key], `${key} must change with the run stamp`)
    assert.ok(!first[key].endsWith('-'), 'names must not end with a separator')
  }
  const names = Object.values(first)
  assert.equal(new Set(names).size, names.length, 'names within one run must be unique')
})

test('cleanup steps revoke grants before uninstalling definitions and dedupe entries', () => {
  const state = {
    installed: [
      { name: 'drill-portable-mcp-x' },
      { name: 'drill-unrelated-stdio-x' },
      { name: 'drill-portable-mcp-x' },
    ],
    granted: [
      { agentRef: 'agent-1', name: 'drill-portable-mcp-x' },
      { agentRef: 'agent-1', name: 'drill-never-installed-x' },
      { agentRef: 'agent-1', name: 'drill-portable-mcp-x' },
    ],
  }

  const steps = drillExtensionCleanupSteps(state)

  assert.deepEqual(steps, [
    { type: 'revoke', agentRef: 'agent-1', kind: 'mcp', name: 'drill-portable-mcp-x' },
    { type: 'revoke', agentRef: 'agent-1', kind: 'mcp', name: 'drill-never-installed-x' },
    { type: 'uninstall', name: 'drill-portable-mcp-x' },
    { type: 'uninstall', name: 'drill-unrelated-stdio-x' },
  ])
  assert.deepEqual(drillExtensionCleanupSteps(null), [])
  assert.deepEqual(drillExtensionCleanupSteps({}), [])
})
