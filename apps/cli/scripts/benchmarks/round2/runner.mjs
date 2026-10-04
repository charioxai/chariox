// MP-08 / MP-10 / MP-11. Wiring for separately authorized round-2 benchmarks.
import { Round2Room } from './room.mjs'
import { waitForSettlement, loadTurnHistory } from './kernel.mjs'
import { writeEvidence, settlementRecord, exportFinalAnswer } from './export.mjs'
import { prepareRound2Prompt } from './policy.mjs'

export async function runRound2Episode({ api, directory, source, roomOptions, roomSetup, provider,
  setup = async () => {}, prompt, questionMode, answerContract, maxMs, guard, observe, audit, grade, verifyCleanup }) {
  const room = new Round2Room(api, { ...roomOptions, checkpoint: owned => writeEvidence(directory, 'ownership.json', owned) })
  const row = { mpItems: ['MP-08', 'MP-10', 'MP-11'], runner: 'round2-shared-v2', source,
    denominatorIncluded: true, status: 'RED', startedAt: new Date().toISOString() }
  let seam = 'room_setup'
  try {
    if (!source?.runtimeCommit || !source?.runnerCommit) throw new Error('MP-10 exact runtime/runner identities required')
    if (!audit || !grade || !verifyCleanup || !roomSetup?.verifySlice || !guard) throw new Error('MP-10 benchmark audit/provenance/resource/cleanup adapters required')
    seam = 'policy_admission'
    const prepared = prepareRound2Prompt({ prompt, startedAt: row.startedAt, questionMode, answerContract })
    row.policy = prepared.policy
    await writeEvidence(directory, 'run.json', row)
    const admissionGuard = async () => {
      if (await guard()) throw new Error('MP-10 resource admission closed')
    }
    seam = 'room_setup'; await room.setup({ ...roomSetup, guard: admissionGuard })
    await admissionGuard()
    await setup(room)
    seam = 'agent_spawn'; await room.spawn(provider)
    // Persist admission before the only submit. Never retry this prompt.
    seam = 'prompt_submit'; row.admission = 'reserved'; await writeEvidence(directory, 'run.json', row)
    const identity = await room.submit(prepared.prompt)
    Object.assign(row, identity, { admission: 'submitted', providerStartedAt: new Date().toISOString() }); await writeEvidence(directory, 'run.json', row)
    seam = 'provider_settlement'
    const settled = await waitForSettlement(api, identity, { maxMs, guard, observe, cancel: () => room.cancel() })
    row.settlement = settlementRecord({ ...settled, ...identity, entries: [], helpers: api.helpers })
    row.observationErrors = settled.observationErrors
    row.historyComplete = false
    if (row.settlement.status === 'RED') row.firstFailingSeam = 'provider_settlement'
    await writeEvidence(directory, 'run.json', row)
    seam = 'history_export'
    const entries = settled.turn ? await loadTurnHistory(api, { ...identity, turn: settled.turn }) : []
    row.historyComplete = !!settled.turn
    row.settlement = settlementRecord({ ...settled, ...identity, entries, helpers: api.helpers })
    if (row.settlement.status === 'RED') {
      row.firstFailingSeam = 'provider_settlement'
    } else {
      seam = 'evaluator_observation'
      if (row.observationErrors.length) throw new Error('MP-10 evaluator observation failed')
      seam = 'tool_audit'; await audit({ room, turn: settled.turn, entries, row, policy: row.policy })
      seam = 'final_response_export'
      const answer = await exportFinalAnswer({ directory, turn: settled.turn, entries, helpers: api.helpers, settlement: row.settlement, answerContract: row.policy.answerContract })
      seam = 'official_scoring'
      row.grade = await grade({ room, answer, policy: row.policy })
      row.status = 'scored'
    }
  } catch (error) {
    row.firstFailingSeam ??= seam
    row.error = { errorClass: error.name, errorCode: error.code ?? null }
  } finally {
    try { row.cleanup = await room.cleanup() }
    catch (error) { row.cleanup = { complete: false, errorClass: error.name, errorCode: error.code ?? null } }
    if (verifyCleanup) {
      try { row.cleanup.external = await verifyCleanup(room.owned) }
      catch (error) { row.cleanup.external = { complete: false, errorClass: error.name, errorCode: error.code ?? null } }
    }
    if (!row.cleanup.complete || row.cleanup.external?.complete !== true) {
      row.status = 'RED'; row.firstFailingSeam ??= 'cleanup'
    }
    if (row.settlement?.cancellation) row.settlement.cancellation.cleanupReceipt = row.cleanup
    row.finishedAt = new Date().toISOString()
    await writeEvidence(directory, 'run.json', row)
  }
  return row
}
