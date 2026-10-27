import assert from 'node:assert/strict'

export const HOSTED_LOCAL_ONLY_REJECTION = {
  rejection: /cannot run local-only extensions/i,
  guidance: /connected ingress/i,
}
export const ACTIVATION_CREDENTIAL_BLOCK = /Required deployment credentials are not ready/i

export function portableHttpsMcpConfig(name, url, options = {}) {
  return {
    name,
    enabled: true,
    required: false,
    transport: {
      type: 'streamable_http',
      url,
      ...(options.bearerTokenCredential ? { bearer_token_credential: options.bearerTokenCredential } : {}),
    },
  }
}

export function localOnlyStdioMcpConfig(name) {
  return {
    name,
    enabled: true,
    required: false,
    transport: { type: 'stdio', command: '/bin/echo', args: [name] },
  }
}

export function assertDrillExtensionName(name) {
  assert.match(name, /^drill-[a-z0-9-]+$/, 'drill extension names must stay namespaced and registry-safe')
  return name
}

export function drillExtensionRunNames(runStamp) {
  assert.ok(/^[a-z0-9-]+$/.test(runStamp), 'run stamp must be registry-safe')
  const withStamp = (base) => assertDrillExtensionName(`${base}-${runStamp}`)
  return {
    grantedPortable: withStamp('drill-portable-mcp'),
    grantedBearer: withStamp('drill-bearer-mcp'),
    grantedLocalOnly: withStamp('drill-local-only-stdio'),
    unrelated: withStamp('drill-unrelated-stdio'),
    missing: withStamp('drill-never-installed'),
  }
}

export function drillExtensionCleanupSteps(state) {
  const steps = []
  const seenRevokes = new Set()
  for (const grant of state?.granted ?? []) {
    const key = `${grant.agentRef}:${grant.name}`
    if (seenRevokes.has(key)) continue
    seenRevokes.add(key)
    steps.push({ type: 'revoke', agentRef: grant.agentRef, kind: 'mcp', name: grant.name })
  }
  const seenUninstalls = new Set()
  for (const definition of state?.installed ?? []) {
    if (seenUninstalls.has(definition.name)) continue
    seenUninstalls.add(definition.name)
    steps.push({ type: 'uninstall', name: definition.name })
  }
  return steps
}

export function expectedMcpRequirement(input) {
  return {
    kind: 'mcp',
    name: input.name,
    agentId: input.agentId,
    nodeIds: input.nodeIds,
    classification: input.classification ?? 'portable',
    credentialSlotCount: input.credentialSlotCount ?? 0,
    networkHost: input.networkHost ?? null,
    expectedUses: [{ agent_id: input.agentId, node_ids: input.nodeIds }],
  }
}

export function assertExtensionRequirementsExact(requirements, expected) {
  assert.equal(requirements.schema_version, 2, 'requirements.json schema_version must be 2')
  assert.ok(Array.isArray(requirements.extensions), 'requirements.json extensions must be an array')
  const actualNames = requirements.extensions.map((extension) => extension.name).sort()
  const expectedNames = [...expected].map((entry) => entry.name).sort()
  assert.deepEqual(actualNames, expectedNames, 'packaged extensions must be exactly the workflow-installed-and-granted set')

  const embeddedSlots = []
  const embeddedDestinations = []
  for (const expectation of expected) {
    const extension = requirements.extensions.find((candidate) => candidate.name === expectation.name)
    assert.ok(extension, `missing packaged requirement for ${expectation.name}`)
    assert.equal(extension.id, `${expectation.kind}:${expectation.name}`)
    assert.equal(extension.kind, expectation.kind)
    assert.match(extension.content_digest, /^sha256:[0-9a-f]{64}$/, `${expectation.name} content_digest must bind exact bytes`)
    assert.equal(extension.version, extension.content_digest, `${expectation.name} version must equal content_digest`)
    assert.deepEqual(extension.uses, expectation.expectedUses, `${expectation.name} must map exact node/agent grant usage`)
    assert.equal(extension.portability?.classification, expectation.classification, `${expectation.name} portability`)
    assert.equal(extension.readiness_test?.kind, expectation.readinessKind ?? 'mcp_initialize', `${expectation.name} readiness test`)
    if (expectation.classification === 'portable') {
      assert.equal(extension.launch_definition?.kind, 'streamable_http', `${expectation.name} portable launch definition`)
    } else {
      assert.equal(extension.launch_definition, null, `${expectation.name} local-only must not carry a launch definition`)
      assert.match(String(extension.portability?.recommendation ?? ''), /connected ingress/i)
    }
    assert.equal(
      (extension.credential_slots ?? []).length,
      expectation.credentialSlotCount,
      `${expectation.name} credential slot count`,
    )
    for (const slot of extension.credential_slots ?? []) {
      assert.match(slot.slot_id, /^integration:/)
      assert.equal(slot.authentication_method, expectation.credentialAuthMethod ?? 'oauth_or_api_key')
      assert.deepEqual(slot.agent_ids.sort(), [...expectation.expectedUses.map((usage) => usage.agent_id)].sort())
    }
    if (expectation.networkHost !== undefined && expectation.networkHost !== null) {
      const destination = (extension.network_destinations ?? []).find((entry) => entry.host?.value === expectation.networkHost)
      assert.ok(destination, `${expectation.name} must declare exact_dns destination ${expectation.networkHost}`)
      assert.equal(destination.host.kind, 'exact_dns')
      assert.deepEqual(destination.ports, [443])
      assert.deepEqual(destination.protocols, ['tls'])
    }
    embeddedSlots.push(...(extension.credential_slots ?? []))
    embeddedDestinations.push(...(extension.network_destinations ?? []))
  }
  assert.deepEqual(requirements.credential_slots, embeddedSlots, 'top-level credential slots must equal flattened extensions')
  assert.deepEqual(
    [...requirements.network_destinations].sort((left, right) => String(left.id).localeCompare(String(right.id))),
    [...embeddedDestinations].sort((left, right) => String(left.id).localeCompare(String(right.id))),
    'top-level network destinations must equal flattened extensions',
  )
}

export function assertUnrelatedExtensionAbsent(requirements, unrelatedNames) {
  for (const name of unrelatedNames) {
    const leak = JSON.stringify(requirements).includes(name)
    assert.ok(!leak, `installed-but-ungranted extension ${name} must be absent from the packaged requirements`)
  }
}

export function assertDeploymentContractMatchesRequirements(contract, requirements) {
  const capabilities = contract?.capabilities
  assert.ok(capabilities, 'deployment contract must embed capabilities')
  assert.deepEqual(capabilities.extensions, requirements.extensions, 'contract extensions must deep-equal requirements extensions')
  const contractSlots = Array.isArray(contract.credential_slots) ? contract.credential_slots : []
  for (const slot of requirements.credential_slots) {
    const mapped = contractSlots.find((candidate) => candidate.slot_id === slot.slot_id)
    assert.ok(mapped, `contract must carry integration credential slot ${slot.slot_id}`)
    assert.equal(mapped.kind, 'integration')
    assert.equal(mapped.required, true, 'extension credential slots must be required until setup binds them')
    assert.equal(mapped.integration, slot.integration)
    assert.equal(mapped.extension_id, slot.extension_id)
  }
  const destinations = capabilities.network?.destinations
  assert.ok(Array.isArray(destinations), 'contract network policy must declare destinations')
  for (const destination of requirements.network_destinations) {
    assert.ok(
      destinations.some((candidate) => candidate.id === destination.id),
      `contract network policy must include ${destination.id}`,
    )
  }
}

export function assertHostedRejectsLocalOnly(errorText) {
  assert.match(String(errorText), HOSTED_LOCAL_ONLY_REJECTION.rejection, 'hosted deployment must reject local-only extensions')
  assert.match(String(errorText), HOSTED_LOCAL_ONLY_REJECTION.guidance, 'rejection must recommend connected ingress')
}

export function assertActivationBlockedBeforeCredentialSetup(errorText) {
  assert.match(
    String(errorText),
    ACTIVATION_CREDENTIAL_BLOCK,
    'activation must block while a required integration credential has no bound setup',
  )
}
