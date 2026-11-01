import assert from 'node:assert/strict'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

function distribution(values) {
  values.sort((a,b) => a-b)
  return { n: values.length, p50_ms: values[Math.ceil(values.length*.5)-1], p95_ms: values[Math.ceil(values.length*.95)-1], max_ms: values.at(-1) }
}

export async function isolationDrills({ rpc, create, root, report, save, connect, relay }) {
  const room = await create(false), unrelated = await create(false)
  const sessionId = room.session.id, otherSessionId = unrelated.session.id
  const terminal = async operation => (await rpc({ RoomTerminal: { session_id: sessionId, operation } })).RoomTerminal.result
  const opened = await terminal({ operation: 'open', cols: 80, rows: 24, command: ['/bin/cat'] })
  const terminalId = opened.terminal.terminal_id
  const file = join(root, 'work', 'probe.bin')
  await rpc({ WriteKernelFile: { path: file, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } })
  let offset = 0, sequence = 0
  async function echo() {
    const text = `echo-${++sequence}`
    await terminal({ operation: 'input', terminal_id: terminalId, data_base64: Buffer.from(text+'\n').toString('base64') })
    let received = ''
    for (let i = 0; i < 50; i++) {
      const result = await terminal({ operation: 'read_output', terminal_id: terminalId, after_offset: offset, wait_ms: 20 })
      offset = result.output.next_offset
      received += Buffer.from(result.output.data_base64, 'base64').toString()
      if (received.includes(text)) return
    }
    throw Error('terminal echo timed out')
  }
  const operations = {
    ListSessions: () => rpc({ ListSessions: {} }),
    ReadKernelFile: () => rpc({ ReadKernelFile: { path: file, offset: 0, length: 1 } }),
    WriteKernelFile: () => rpc({ WriteKernelFile: { path: file, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } }),
    TerminalEcho: echo,
    GetSessionState: () => rpc({ GetSessionState: { session_id: otherSessionId } }),
  }
  async function measure(rounds) {
    const times = Object.fromEntries(Object.keys(operations).map(kind => [kind, []]))
    for (let i = 0; i < rounds; i++) for (const [kind, operation] of Object.entries(operations)) {
      const at = performance.now(); await operation(); times[kind].push(performance.now()-at)
    }
    return Object.fromEntries(Object.entries(times).map(([kind, values]) => [kind, distribution(values)]))
  }
  const idle = await measure(30)
  report.isolation = { idle }; save()
  if (process.env.KR_SKIP_SYNC_STALL !== '1') {
    writeFileSync(join(root, 'stall-armed'), '')
    const at = performance.now()
    let mutationAt
    const mutation = create(false).then(value => { mutationAt = performance.now(); return value })
    mutation.catch(() => {})
    for (let i = 0; !existsSync(join(root, 'stall-started')); i++) {
      assert(i < 1000, 'sync injection was not reached'); await delay(5)
    }
    const during = await measure(30)
    const created = await mutation
    const start = Number(readFileSync(join(root, 'stall-started'), 'utf8')) * 1000
    const end = Number(readFileSync(join(root, 'stall-ended'), 'utf8')) * 1000
    const now = performance.now(), monoNow = Number(process.hrtime.bigint()) / 1e6
    const mutationMono = monoNow - (now - mutationAt)
    report.isolation.fsync = { injected_ms: end-start, mutation_wait_ms: mutationAt-at, mutation_after_stall_ms: mutationMono-end, unrelated: during,
      gates: Object.fromEntries(Object.entries(during).map(([kind, result]) => [kind, { pass: result.p95_ms <= idle[kind].p95_ms+50, threshold_ms: idle[kind].p95_ms+50 }])) }
    save()
    await rpc({ EndSession: { session_id: created.session.id } })
  }
  const floodMarker = join(root, 'terminal-flood-started')
  const flood = await terminal({ operation: 'open', cols: 80, rows: 24,
    command: ['python3', '-c', `import sys,pathlib,time; pathlib.Path(${JSON.stringify(floodMarker)}).write_text('started'); [(sys.stdout.buffer.write(b"x"*1_000_000),sys.stdout.flush(),time.sleep(.02)) for _ in range(50)]`] })
  for (let i = 0; !existsSync(floodMarker); i++) { assert(i < 1000, 'terminal flood did not start'); await delay(5) }
  const duringFlood = await measure(30)
  const floodWait = await terminal({ operation: 'wait', terminal_id: flood.terminal.terminal_id, watches: [{kind:'finished'}], timeout_ms: 60000 })
  report.isolation.terminal_flood = { bytes: 50_000_000, unrelated: duringFlood, finished: floodWait.waited.fired, output_bytes: floodWait.waited.terminal.output_offset,
    gates: Object.fromEntries(Object.entries(duringFlood).map(([kind, result]) => [kind, { pass: result.p95_ms <= idle[kind].p95_ms+50, threshold_ms: idle[kind].p95_ms+50 }])) }
  save()
  await terminal({ operation: 'close', terminal_id: flood.terminal.terminal_id })
  if (process.env.KR_CODEX_PROFILE && process.env.KR_PROVIDER_FLOOD === '1') {
    const { floodProvider } = await import('./provider-flood.mjs')
    const flooded = await floodProvider({rpc,root,connect,measure,profilePath:process.env.KR_CODEX_PROFILE,profileId:report.provider_load?.profile_id})
    flooded.gates=Object.fromEntries(Object.entries(flooded.unrelated).map(([kind,result])=>[kind,{pass:result.p95_ms<=idle[kind].p95_ms+50,threshold_ms:idle[kind].p95_ms+50}]))
    report.isolation.provider_flood=flooded;save()
  }
  const slow = connect()
  try {
    const attached = (await rpc({ AttachToSession: { session_id: otherSessionId, client_id: 'slow-reader', capability_level: 'FullTerminal' } })).SessionAttached
    await slow.subscribeToKernelEvents(otherSessionId, attached.attachment.id)
    const socket = slow.eventWebsocket?._socket
    assert(socket, 'dedicated event socket required')
    socket.pause()
    let mutations = 0, failure, finished = false
    const producer = (async () => {
      for (let i=0;i<10000;i++) {
        await rpc({ AliasSession: { session_id: otherSessionId, alias: `slow-reader-${i}` } })
        mutations++
      }
    })().catch(error => { failure=String(error) }).finally(() => { finished=true })
    for(let i=0;socket.readableLength < socket.readableHighWaterMark && !finished && i<200;i++) await delay(50)
    const buffered=socket.readableLength, highWaterMark=socket.readableHighWaterMark, atStart=mutations
    const during=await measure(30)
    socket.resume()
    await producer
    report.isolation.slow_reader={mutations,mutations_at_measurement:atStart,buffered_bytes:buffered,high_water_mark:highWaterMark,producer_failure:failure,unrelated:during,gates:Object.fromEntries(Object.entries(during).map(([kind,result])=>[kind,{pass:result.p95_ms<=idle[kind].p95_ms+50,threshold_ms:idle[kind].p95_ms+50}]))}
    save()
    await rpc({ DetachFromSession: { session_id: otherSessionId, attachment_id: attached.attachment.id } })
  } finally { await slow.close() }
  if (relay) {
    const { disconnectPeer } = await import('./relay.mjs')
    report.isolation.relay_disconnect = await disconnectPeer({ ...relay, rpc, measure })
    report.isolation.relay_disconnect.gates = Object.fromEntries(Object.entries(report.isolation.relay_disconnect.unrelated).map(([kind, result]) => [kind, { pass: result.p95_ms <= idle[kind].p95_ms+50, threshold_ms: idle[kind].p95_ms+50 }]))
    save()
  }
  await terminal({ operation: 'close', terminal_id: terminalId })
  await rpc({ EndSession: { session_id: sessionId } })
  await rpc({ EndSession: { session_id: otherSessionId } })
}
