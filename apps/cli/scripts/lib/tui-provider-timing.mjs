// MP-08 / MP-10: bounded timing of only the drill-owned official provider.
// The sidecar discards protocol payloads and persists only allowlisted metadata.
import assert from 'node:assert/strict'
import path from 'node:path'
import { readFile, writeFile, readdir, readlink } from 'node:fs/promises'

export async function observeProviderTiming({ client, alias, evidence, stop, kernelPid }) {
  assert.ok(alias.startsWith('tuifix-drill-'))
  assert.ok(Number.isInteger(kernelPid) && kernelPid > 1)
  const resolved = Object.values(await client.send({ ResolveSession: { session_ref: alias, workspace_id: null } }))[0]
  const sessionId = resolved.session.id, cwd = process.cwd()
  const candidates = async () => {
    const pids=[]
    for (const name of await readdir('/proc')) {
      if (!/^\d+$/.test(name)) continue
      try {
        if ((await readFile(`/proc/${name}/comm`, 'utf8')).trim() !== 'claude') continue
        if (await readlink(`/proc/${name}/cwd`) !== cwd) continue
        const status=await readFile(`/proc/${name}/status`, 'utf8')
        if (Number(status.match(/^PPid:\s+(\d+)/m)?.[1]) === kernelPid) pids.push(Number(name))
      } catch (error) { if (error.code !== 'ENOENT' && error.code !== 'ESRCH') throw error }
    }
    return pids
  }
  const existing = new Set(await candidates())
  const timing = { items:['MP-08','MP-10'], observer:'owned-provider-protocol-metadata-v3', sessionId,
    dispatchRequestedMs:null, kernelAcceptedMs:null, dispatchReceivedMs:null, firstProviderByteMs:null,
    firstKernelOutputMs:null, kernelMarkerMs:null, firstRenderedOutputMs:null, attachMs:null, pid:null, samples:[], events:[] }
  const unsubscribe=client.onKernelEvent(event=>{
    if(!timing.dispatchRequestedMs)return
    if(event.event==='provider_run_changed'){
      if(timing.events.length<200)timing.events.push({atMs:Date.now(),kind:event.event,state:event.provider_run?.state})
      return
    }
    if(event.event==='runtime_notices'){
      if(timing.events.length<200)timing.events.push({atMs:Date.now(),kind:event.event,count:event.notices.length})
      return
    }
    if(event.event!=='terminal_output')return
    for(const record of event.records){
      const hasMarker=Buffer.from(record.bytes??[]).includes(Buffer.from('TUIFIX MARKER'))
      const now=Date.now()
      if(record.kind==='provider_output'){
        timing.firstKernelOutputMs??=now
        if(hasMarker)timing.kernelMarkerMs??=now
      }
      if(timing.events.length<200)timing.events.push({atMs:now,timestampMs:record.timestamp_ms,kind:record.kind,byteCount:record.bytes?.length??0,hasMarker})
    }
  })
  // Each event consumer needs its own attachment; reusing the TUI's would
  // compete for that attachment's output queue and alter the measured path.
  const attached=Object.values(await client.send({AttachToSession:{session_id:sessionId,client_id:`${alias}-timing`,capability_level:'FullTerminal'}}))[0]
  const attachmentId=attached.attachment.id
  timing.observerAttachmentId=attachmentId
  await client.subscribeToKernelEvents(sessionId,attachmentId)
  let tracer, lastProbe=0, finished=false, poll, inFlight, pollError
  const tick=async()=>{
    if (Date.now()-lastProbe<(tracer?100:10)) return
    lastProbe=Date.now()
    if(tracer)return
    const owned=(await candidates()).filter(pid=>!existing.has(pid))
    assert.ok(owned.length<=1,'one new Claude process in this reserved worktree')
    if (!owned.length) return
    const pid=owned[0]
    assert.ok(Number.isInteger(pid) && pid>1)
    tracer=Bun.spawn(['python3',new URL('./tui-provider-protocol-observer.py',import.meta.url).pathname,String(pid),path.join(evidence,'provider-protocol.jsonl')],{stdout:'ignore',stderr:'ignore'})
    timing.pid=pid;timing.attachMs=Date.now()
  }
  const probe=()=>{
    if(!inFlight)inFlight=tick().finally(()=>{inFlight=null})
    return inFlight
  }
  return {
    dispatch(){
      timing.dispatchRequestedMs=Date.now()
      // Discover the lazy provider while the client captures its first frame.
      poll=setInterval(()=>{
        if(inFlight || finished)return
        probe().catch(error=>{pollError=error})
      },10)
    },
    rendered(){timing.firstRenderedOutputMs=Date.now()},
    tick:probe,
    async sample(){
      await probe()
      const session=Object.values(await client.send({GetSessionState:{session_id:sessionId}}))[0].session
      // Explicit allowlist: no prompt, terminal record, interaction or credential.
      timing.samples.push({atMs:Date.now(),activePrompt:Boolean(session.active_prompt),lastPromptSentMs:session.last_prompt_sent_at_ms,
        agents:session.agents.map(agent=>({id:agent.id,state:agent.state,processing:agent.is_processing})),
        activeInteractions:session.active_interactions?.length??0,queued:session.queued_prompts?.length??0})
      await writeFile(path.join(evidence,'provider-timing-progress.json'),JSON.stringify(timing,null,2))
    },
    async finish(){
      if(finished)return timing
      finished=true
      clearInterval(poll)
      await inFlight
      unsubscribe()
      try {
        await client.unsubscribeFromKernelEvents()
        await client.send({DetachFromSession:{attachment_id:attachmentId}})
      } finally {
        if(tracer){assert.ok(Number.isInteger(tracer.pid)&&tracer.pid>1);tracer.kill('SIGINT');await Promise.race([tracer.exited,Bun.sleep(3000)]);await stop(tracer)}
      }
      if(pollError)timing.observerError=pollError.message
      timing.kernelAcceptedMs=timing.samples.find(s=>s.lastPromptSentMs>=timing.dispatchRequestedMs)?.lastPromptSentMs??null
      const protocolFile=path.join(evidence,'provider-protocol.jsonl')
      const protocol=(await readFile(protocolFile,'utf8')).trim().split('\n').map(line=>JSON.parse(line)).filter(row=>row.type)
      timing.providerProtocol=protocol
      assert.ok(protocol.length,'provider observer must capture startup records')
      timing.dispatchReceivedMs=protocol.find(row=>row.submittedPromptSeen)?.atMs??null
      timing.firstProviderByteMs=protocol[0]?.atMs??null
      timing.providerMarkerMs=protocol.find(row=>row.markerSeen)?.atMs??null
      timing.limitations='TUI Enter -> kernel last_prompt_sent -> first complete official stdout JSON record -> provider replay of our prompt -> kernel provider_output -> rendered marker (50 ms polling). Exact parent/new PID/cwd/thread/stdout-pipe ownership. Separate observer attachment. Payloads discarded in memory. Lazy-spawn attachment may miss earlier writes; the prompt replay is an upper bound on input receipt, not its stdin-read timestamp.'
      await writeFile(path.join(evidence,'provider-timing.json'),JSON.stringify(timing,null,2))
      return timing
    },
  }
}
