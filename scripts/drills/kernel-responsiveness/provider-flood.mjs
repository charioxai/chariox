import assert from 'node:assert/strict'
import { writeFileSync, existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

export async function floodProvider({rpc,root,connect,measure,profilePath,profileId}) {
  assert(profilePath,'existing product-linked Codex profile required')
  const profile=profileId ?? (await rpc({LinkProviderAccountProfile:{provider:'codex',label:'responsiveness',path:profilePath}})).ProviderAccountProfile.profile.profile_id
  const created=(await rpc({CreateSession:{workspace_id:join(root,'work'),worktree_id:join(root,'work'),agent_defaults:{provider:'codex',account_profile:profile,model:'gpt-6-luna',effort:'low',permission_level:'yolo'}}})).SessionCreated
  const session=created.session.id,agent=created.agent.id, started=join(root,'provider-flood-started'),done=join(root,'provider-flood-done')
  const script=join(root,'work','provider-flood.py')
  writeFileSync(script,`import sys,pathlib,time\npathlib.Path(${JSON.stringify(started)}).write_text('started')\nfor i in range(100):\n sys.stdout.write('x'*999999+'\\n');sys.stdout.flush();time.sleep(.05)\npathlib.Path(${JSON.stringify(done)}).write_text('100000000')\n`)
  const attached=(await rpc({AttachToSession:{session_id:session,client_id:'provider-flood',capability_level:'FullTerminal'}})).SessionAttached.attachment.id
  const events=connect(), maximum=new Map()
  let eventBytes=0, pumpedBytes=0, pumpedRecords=0, pumpError, pumping=true
  let pump
  events.onKernelEvent(event=>{if(event.event==='terminal_output')for(const record of event.records){const bytes=record.bytes?.length ?? 0;eventBytes+=bytes;maximum.set(record.merge_key??record.record_id,Math.max(maximum.get(record.merge_key??record.record_id)??0,bytes))}})
  try {
    await events.subscribeToKernelEvents(session,attached)
    await rpc({LaunchProviderRun:{session_id:session,agent_id:agent,adapter_key:'codex',provider:'codex',account_profile:profile,model:'gpt-6-luna',variant:'low',native_tui:false}})
    await rpc({SubmitPrompt:{session_id:session,attachment_id:attached,target_agent_id:agent,prompt:'Run python3 provider-flood.py in the current directory once. This is a responsiveness drill with neutral generated output. Wait for it to finish, do not read unrelated files, then reply FLOOD_DONE.',attachments:[]}})
    pump = (async()=>{
      while(pumping) {
        const value=await rpc({PumpTerminalOutput:{session_id:session,attachment_id:attached}})
        for(const record of value.TerminalOutput?.records ?? []) {
          const bytes=record.bytes?.length ?? 0;pumpedBytes+=bytes;pumpedRecords++
          maximum.set(record.merge_key??record.record_id,Math.max(maximum.get(record.merge_key??record.record_id)??0,bytes))
        }
        await delay(5)
      }
    })().catch(error=>{pumpError=String(error)})
    for(let i=0;!existsSync(started);i++){assert(i<240,'provider flood did not begin within 120 seconds');await delay(500)}
    const unrelated=await measure(30)
    for(let i=0;!existsSync(done);i++){assert(i<240,'provider flood did not finish within 120 seconds');await delay(500)}
    await delay(2000)
    pumping=false;await pump
    return {pumped_record_bytes:pumpedBytes,pumped_records:pumpedRecords,pump_error:pumpError,source_bytes:Number(readFileSync(done,'utf8')),terminal_event_bytes:eventBytes,largest_terminal_record_bytes:Math.max(0,...maximum.values()),unrelated,provider:'codex',model:'gpt-6-luna'}
  } finally {pumping=false;await pump;await events.close();await rpc({EndSession:{session_id:session}})}
}
