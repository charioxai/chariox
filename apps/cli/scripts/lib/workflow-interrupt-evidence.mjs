// MP-08 / MP-10: retain only provider timing/identity metadata, never payloads.
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'

export async function interruptTraces(logDir) {
  const traces = []
  for (const name of await readdir(logDir).catch(() => [])) {
    if (!name.endsWith('.ndjson')) continue
    for (const line of (await readFile(path.join(logDir, name), 'utf8')).split('\n')) {
      let entry; try { entry = JSON.parse(line) } catch { continue }
      if(entry.component==='daemon.command_latency' && entry.command_type==='room_workflow_runs.control') {
        traces.push({at:entry.timestamp_ms,message:entry.message,commandType:entry.command_type,source:entry.source,queueWaitMs:entry.queue_wait_ms,commandAgeMs:entry.command_age_ms})
        continue
      }
      if (entry.component !== 'daemon.provider.codex'  || ![
        'codex turn interrupt sent trace', 'codex turn completion received trace',
        'codex command output received trace', 'codex thread terminals cleaned trace',
      ].includes(entry.message)) continue
      traces.push({ at: entry.timestamp_ms, message: entry.message,
        providerRunId: entry.provider_run_id, turnId: entry.turn_id,
        status: entry.status, outputBytes: entry.output_bytes })
    }
  }
  return traces.sort((a, b) => a.at - b.at)
}

export function interruptTiming(traces, sentAtMs) {
  const sent = traces.find(trace => trace.at >= sentAtMs && trace.message === 'codex turn interrupt sent trace')
  const lastSent = sent && traces.filter(trace => trace.at >= sent.at && trace.providerRunId === sent.providerRunId
    && trace.message === 'codex turn interrupt sent trace').at(-1)
  const ended = lastSent && traces.find(trace => trace.at >= lastSent.at && trace.turnId === lastSent.turnId
    && trace.providerRunId === sent.providerRunId && trace.message === 'codex turn completion received trace')
  return { sent, lastSent, ended, stopToInterruptSentMs: sent && sent.at - sentAtMs,
    stopToTurnEndedMs: ended && ended.at - sentAtMs }
}
