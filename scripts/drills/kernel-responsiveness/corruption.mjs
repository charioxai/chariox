// Deliberate replay corruption, confined to a stopped disposable kernel's database.
import assert from 'node:assert/strict'
import { DatabaseSync } from 'node:sqlite'
import { randomUUID } from 'node:crypto'
import { join } from 'node:path'

export function injectMissingAgents(root, sessions, mismatches) {
  assert(root.startsWith('/root/.chariox/dev/') && sessions.length > 0)
  const db = new DatabaseSync(join(root, 'chariox/state.db'))
  try {
    const insert = db.prepare('INSERT INTO durable_state_events(event_id,kind,subject_id,timestamp_ms,payload_json) VALUES(?,?,?,?,?)')
    db.exec('BEGIN IMMEDIATE')
    for (let i = 0; i < sessions.length; i++) {
      const session = structuredClone(sessions[i])
      assert(session.agents.length > 0, 'real canonical session agent required')
      const count = Math.floor(mismatches/sessions.length) + (i < mismatches%sessions.length ? 1 : 0)
      for (let j=0;j<count;j++) session.agents.push({...session.agents[0],id:randomUUID(),agent_ref:`missing-${i}-${j}`})
      insert.run(randomUUID(),'session.updated',session.id,Date.now(),JSON.stringify({session}))
    }
    db.exec('COMMIT')
  } finally {db.close()}
}

export async function measureCorruptHealth({rpc, report, save, expected, samples=100, root, distribution}) {
  const health = [], other = [], sizes = []
  const path = join(root,'work','health-probe')
  await rpc({WriteKernelFile:{path,offset:0,total_size:1,data_base64:'eA==',overwrite:true}})
  const idle=[]
  for(let i=0;i<samples;i++) {const at=performance.now();await rpc({ReadKernelFile:{path,offset:0,length:1}});idle.push(performance.now()-at)}
  let counts
  for(let i=0;i<samples;i++) {
    await Promise.all([
      (async()=>{const at=performance.now();const value=await rpc({GetDaemonHealth:null});health.push(performance.now()-at);sizes.push(Buffer.byteLength(JSON.stringify(value)));counts=value.DaemonHealth.projection.projection_invariants})(),
      (async()=>{const at=performance.now();await rpc({ReadKernelFile:{path,offset:0,length:1}});other.push(performance.now()-at)})(),
    ])
  }
  const count = counts.mismatch_count ?? counts.mismatches.length
  report.corrupt_health={expected_mismatches:expected,actual_mismatches:count,checked_sessions:counts.checked_sessions,sample:counts.mismatches.length,counts_by_kind:counts.mismatch_counts_by_kind,max_bytes:Math.max(...sizes),health:distribution(health),other_idle:distribution(idle),other_during:distribution(other)}
  report.corrupt_health.gates={mismatch_fixture:count===expected,size:report.corrupt_health.max_bytes<=65536,time:report.corrupt_health.health.p95_ms<=100,other:report.corrupt_health.other_during.p95_ms<=report.corrupt_health.other_idle.p95_ms+50}
  save()
}
