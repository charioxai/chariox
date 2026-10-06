// MP-08/MP-10/MP-11: immutable settled receipts fence every previously attempted task.
import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { verifyCleanupSettlement } from './webvoyager-cleanup-settlement.mjs'

export function assertRuntimeClosed(cleanup) {
  assert(cleanup?.stateRemoved && cleanup.workspaceRemoved && Array.isArray(cleanup.unresolvedRooms)
    && !cleanup.unresolvedRooms.length && Array.isArray(cleanup.processes) && cleanup.processes.length
    && (!('unresolvedSlices' in cleanup) || (Array.isArray(cleanup.unresolvedSlices) && !cleanup.unresolvedSlices.length))
    && cleanup.processes.every(p => p.stopped), 'MP-11 prior runtime cleanup incomplete')
}

export function assertSameController(prior, current) {
  for (const key of ['image', 'runtimeSourceRevision', 'protocol', 'clientIpcSha256']) {
    assert(prior?.[key], `MP-10 prior ${key} missing`)
    assert.equal(current[key], prior[key], `MP-10 controller ${key} changed`)
  }
  assert.deepEqual(current.artifacts, prior.artifacts, 'MP-10 runtime artifacts changed')
}

export async function loadContinuation({ priorEvidence, newEvidence, selection, cleanupSettlement, scope = 'round3-full', observationPolicy }) {
  assert(path.isAbsolute(priorEvidence) && path.resolve(priorEvidence) !== path.resolve(newEvidence), 'MP-10 preserve original evidence')
  const names = ['CAMPAIGN.json', 'RESULTS.json', 'FULL_SELECTION.json']
  const bytes = await Promise.all(names.map(name => readFile(`${priorEvidence}/${name}`)))
  const [campaign, rows, priorSelection] = bytes.map(buffer => JSON.parse(buffer))
  assert(campaign.finishedAt && !campaign.completed, 'MP-10 continuation requires settled, incomplete campaign')
  if (observationPolicy) {
    assert.equal(scope, 'round4-full'); assert.equal(campaign.scope, scope)
    assert.equal(observationPolicy, 'vision-allowed'); assert.equal(campaign.observationPolicy, observationPolicy)
    assert.equal(campaign.promptVariant, 'recovery-vision-v1')
    assert(!rows.some(row => row.providerUnauthorized || row.providerUsageExhausted || row.judgeFailure?.unauthorized || row.judgeFailure?.usageExhausted),
      'MP-08/MP-10 quota/auth stop requires new owner authorization; never auto-continue')
    assert(rows.every(row => row.observationPolicy === observationPolicy), 'MP-10 observation policies must not mix')
  }
  const settlement = cleanupSettlement ? await verifyCleanupSettlement({ file: cleanupSettlement, priorEvidence, bytes, rows }) : null
  if (settlement) assertSameController(campaign.source, settlement.source)
  else assertRuntimeClosed(campaign.cleanup)
  assert.deepEqual(priorSelection, selection, 'MP-10 frozen selection changed')
  for (const [key, value] of Object.entries({ model: 'gpt-6.1-sol', judgeModel: 'gpt-6.1-sol', effort: 'high', judgeEffort: 'low', maxMutatingActions: 15, maxToolCalls: 80, wallTimeoutMs: 600000 })) {
    assert.equal(campaign[key], value, `MP-10 frozen ${key} changed`)
  }
  const selected = new Map(selection.tasks.map(task => [task.id, task])), seen = new Set()
  for (const row of rows) {
    const task = selected.get(row.taskId)
    assert(task && !seen.has(row.taskId), 'MP-10 foreign or duplicate prior task')
    seen.add(row.taskId)
    assert.equal(!!row.excluded, !!task.excludedReason, 'MP-10 exclusion changed')
    if (row.excluded) assert.equal(row.excludedReason, task.excludedReason)
    else assert(row.cleanupValid || settlement?.settled.has(row.taskId), 'MP-11 prior attempt cleanup incomplete')
  }
  assert.equal(rows.filter(row => !row.excluded).length, campaign.settled)
  assert.equal(rows.filter(row => row.excluded).length, 11)
  const admissionHashes = []
  for (const file of (await readdir(`${priorEvidence}/admissions`)).sort()) {
    assert(file.endsWith('.json'), 'MP-10 unexpected admission artifact')
    const data = await readFile(`${priorEvidence}/admissions/${file}`), receipt = JSON.parse(data)
    assert(seen.has(receipt.taskId) && !selected.get(receipt.taskId).excludedReason, 'MP-10 ambiguous admission without settled row; never replay')
    assert.equal(file, `${scope}-${encodeURIComponent(receipt.taskId)}.json`)
    admissionHashes.push({ file, sha256: createHash('sha256').update(data).digest('hex') })
  }
  const tasks = selection.tasks.filter(task => !task.excludedReason && !seen.has(task.id))
  assert(tasks.length, 'MP-10 no unattempted tasks remain')
  return { rows, tasks, source: campaign.source, provenance: { priorEvidence,
    priorReceipts: names.map((name, i) => ({ name, sha256: createHash('sha256').update(bytes[i]).digest('hex') })),
    ...(observationPolicy ? { observationPolicy, resourceSha256: createHash('sha256').update(await readFile(`${priorEvidence}/runtime-resources.jsonl`)).digest('hex') } : {}),
    admissionHashes, priorSettled: campaign.settled, unattempted: tasks.length, solverRetries: 0,
    ...(settlement ? { cleanupSettlement: settlement.provenance } : {}) } }
}
