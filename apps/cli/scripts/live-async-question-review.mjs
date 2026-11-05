// MP-08 / MP-09 / MP-10 / MP-11: review regressions on the real TUI/provider path.
import assert from 'node:assert/strict'
import {readFile,readdir,readlink} from 'node:fs/promises'
import {execFileSync} from 'node:child_process'
import path from 'node:path'
import {ownedProcessHandles,signalOwnedProcess,processSnapshot} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

async function replayCount(logDir, id) {
  let count=0
  for(const name of await readdir(logDir)) {
    const file=path.join(logDir,name)
    if(!name.endsWith('.ndjson'))continue
    for(const line of (await readFile(file,'utf8')).split('\n')) {
      let record;try{record=JSON.parse(line)}catch{continue}
      if(record.component==='runtime.interaction_wake'&&record.message==='reconciling finished question'&&record.interaction_id===id)count++
    }
  }
  return count
}
function taskEvidence(owner,id) {
  const script='import sqlite3,json,sys\nc=sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True)\nr=c.execute("SELECT payload FROM agent_tasks WHERE task_id=?",(sys.argv[2],)).fetchone()\nprint(r[0] if r else "null")'
  return JSON.parse(execFileSync('python3',['-c',script,path.join(owner.env.CHARIOX_HOME,'state/kernel.db'),id],{encoding:'utf8'}))
}

export async function reviewQuestions({owner,root,until,sleep,capture,record,questionEvidence,report}) {
  const results=[]
  report.review=results
  const phase=process.env.AMASK_REVIEW_PHASE??'all'
  assert(['all','busy','deadline','timing','failure'].includes(phase),'Unknown review phase')
  report.review_phase=phase
  if(phase==='all'||phase==='busy'){
  const marker=path.join(root,'busy-marker')
  await owner.tui.send('submit_prompt',{prompt:`Call chariox.request_popup with wait:false, title "AMASK review busy", message "Answer while work continues", choices [{id:"continue",label:"Continue",reply:"AMASK busy answer"}], timeout_sec:300. Then run this exact shell command and wait for it: touch '${marker}'; sleep 90. After it finishes print AMASK_BUSY_DONE. When later woken, read events.inbox, handle the answer and acknowledge it as handled.`})
  const busy=await until('busy question',owner.session,s=>s.active_interactions?.some(q=>q.title==='AMASK review busy'))
  const q=busy.active_interactions.find(q=>q.title==='AMASK review busy')
  await until('official provider shell busy',()=>readFile(marker).then(()=>true).catch(()=>false))
  await capture(owner.tui,'review-01-busy-question')
  await owner.tui.send('interaction_submit',{choiceIndex:0})
  await until('answer recorded while provider busy',()=>questionEvidence(owner,q.id),r=>r.answers.length===1)
  const before=await replayCount(owner.env.CHARIOX_LOG_DIR,q.id)
  const started=Date.now()
  await sleep(65_000)
  const after=await replayCount(owner.env.CHARIOX_LOG_DIR,q.id)
  const idle={elapsed_ms:Date.now()-started,finished_cleanup_attempts:after-before,...questionEvidence(owner,q.id)}
  assert.equal(idle.wake.fire_count,1)
  assert.equal(idle.answers.length,1)
  await record('review-02-finished-counter',idle)
  results.push({finding:'finished wake replay',pass:after-before===0,attempts:after-before})
  await capture(owner.tui,'review-02-busy-after-minute')
  await until('busy answer task handled',owner.session,s=>s.agent_tasks?.some(t=>t.task_id===q.asking_task_id&&t.state==='done'),120_000)

  }
  if(phase==='all'||phase==='deadline'){
  await owner.tui.send('submit_prompt',{prompt:'Call chariox.request_popup twice, wait:false for both. First title "AMASK review long", message "Ten day deadline", choices [{id:"continue",label:"Continue",reply:"Long answer"}], timeout_sec:864000. Second title "AMASK review unbounded", message "No timeout", same choices, omit timeout_sec. Do not call events.yield or create an explicit wait. End this turn normally with the final text AMASK_LONG_ARMED; automatic question retention is what this drill tests. Leave both questions pending. When later woken, read events.inbox, handle and acknowledge the answers, and finish.'})
  const long=await until('long questions retained',owner.session,s=>{
    const qs=(s.active_interactions??[]).filter(q=>q.title?.startsWith('AMASK review '))
    return qs.length===2&&s.agent_tasks?.some(t=>t.task_id===qs[0].asking_task_id&&t.state==='waiting')
  })
  const questions=long.active_interactions.filter(q=>q.title?.startsWith('AMASK review '))
  const task=taskEvidence(owner,questions[0].asking_task_id)
  await record('review-03-retained-deadlines',{questions,task})
  assert.equal(task.reason,'Waiting for the answer to a non-blocking question','This drill requires normal settlement, not an explicit provider yield')
  results.push({finding:'question deadline',pass:task.wait.deadline_ms===253_402_300_799_999,deadline_ms:task.wait.deadline_ms})
  await capture(owner.tui,'review-03-long-questions')
  assert(!JSON.stringify(await owner.tui.send('snapshot')).includes('Invalid wait deadline'),'The retained deadline must render in the real TUI')
  for(const question of questions) {
    await until('long question visible for answer',()=>owner.tui.send('snapshot'),s=>s.interactions?.some(q=>q.id===question.id))
    await owner.tui.send('interaction_submit',{choiceIndex:0})
    await until('long question answered',owner.session,s=>!(s.active_interactions??[]).some(q=>q.id===question.id))
  }
  await until('long question answers handled',owner.session,s=>s.agent_tasks?.some(t=>t.task_id===questions[0].asking_task_id&&t.state==='done'),120_000)

  }
  if(phase==='all'||phase==='timing'){
  await owner.tui.send('submit_prompt',{prompt:'For this timer/question deadline drill call chariox.events.timer with delay_ms:3000, label "AMASK review timer" and this turn task metadata. Call chariox.request_popup twice with wait:false, titles "AMASK review timeout default" and "AMASK review timeout empty", timeout_sec:10, choices [{id:"continue",label:"Continue",reply:"AMASK review timed out"}]. Only the first has default_on_timeout:"continue". Then yield with all three registration ids, this turn metadata, and a finite deadline 60 seconds from now. When woken read events.inbox, report and handle every received timer/question outcome, acknowledge each as handled, and yield again on any remaining live source. Finish once all three outcomes are handled.'})
  const timed=await until('independent timer and both question deadlines',owner.session,s=>{
    const wakes=(s.agent_wakes??[]).filter(w=>w.label==='AMASK review timer'||w.detail?.title?.startsWith('AMASK review timeout '))
    return wakes.length===3&&wakes.every(w=>w.fire_count===1)
  },120_000)
  const timedWakes=timed.agent_wakes.filter(w=>w.label==='AMASK review timer'||w.detail?.title?.startsWith('AMASK review timeout '))
  const timedQuestions=timedWakes.filter(w=>w.kind==='interaction').map(w=>({wake:w,answers:questionEvidence(owner,w.id).answers}))
  for(const item of timedQuestions) {
    assert.equal(item.answers.length,1)
    assert.equal(item.answers[0].status,'timed_out')
    assert.equal(item.answers[0].reply,item.wake.detail.title.endsWith(' default')?'AMASK review timed out':null)
  }
  await until('timer and question outcomes handled',owner.session,s=>s.agent_tasks?.some(t=>t.task_id===timedWakes[0].task_id&&t.state==='done'),120_000)
  await capture(owner.tui,'review-04-timer-and-question-deadlines')
  await record('review-04-timer-and-question-deadlines',{wakes:timedWakes,questions:timedQuestions})
  report.cells.push({id:'AM-ASK-04',result:'REVIEW PASS',independent_timer:true,default_and_empty_timeout:true})

  }
  if(phase==='all'||phase==='failure'){
  const failureMarker=path.join(root,'failure-marker')
  await owner.tui.send('submit_prompt',{prompt:`Call chariox.request_popup with wait:false, title "AMASK review failure", message "Provider failure must stay visible", choices [{id:"continue",label:"Continue",reply:"Failure answer"}], timeout_sec:300. Then run this exact shell command and wait for it: touch '${failureMarker}'; sleep 90. Leave the question pending.`})
  const failure=await until('failure question',owner.session,s=>s.active_interactions?.some(q=>q.title==='AMASK review failure'))
  const failedQuestion=failure.active_interactions.find(q=>q.title==='AMASK review failure')
  await until('official provider failure shell busy',()=>readFile(failureMarker).then(()=>true).catch(()=>false))
  const candidates=[]
  for(const handle of ownedProcessHandles(owner.child)) {
    if(!Number.isSafeInteger(handle.pid)||handle.pid<=1)throw Error('Invalid owned provider PID')
    const executable=await readlink('/proc/'+handle.pid+'/exe').catch(()=>null)
    if(executable&&path.basename(executable)==='codex')candidates.push(handle)
  }
  const processes=processSnapshot()
  const parents=new Map(processes.map(p=>[p.pid,p.ppid]))
  const ancestors=pid=>{
    const ids=new Set()
    for(let parent=parents.get(pid);parent>1&&!ids.has(parent);parent=parents.get(parent))ids.add(parent)
    return ids
  }
  const sleepers=[]
  for(const handle of ownedProcessHandles(owner.child)) {
    const executable=await readlink('/proc/'+handle.pid+'/exe').catch(()=>null)
    if(executable&&path.basename(executable)==='sleep')sleepers.push(handle.pid)
  }
  const active=candidates.filter(h=>sleepers.some(pid=>ancestors(pid).has(h.pid)))
    .filter(h=>!candidates.some(other=>ancestors(h.pid).has(other.pid)))
  await record('review-05-provider-ownership',{candidates:candidates.map(h=>({pid:h.pid,ppid:parents.get(h.pid)})),sleepers,active:active.map(h=>h.pid)})
  assert.equal(active.length,1,'Identify exactly one official Codex ancestor of the busy shell in this owned kernel subtree')
  signalOwnedProcess(active[0],'SIGKILL')
  const ended=await until('provider failure settlement',owner.session,s=>s.agent_tasks?.some(t=>t.task_id===failedQuestion.asking_task_id&&t.state!=='working'),90_000)
  const failedTask=ended.agent_tasks.find(t=>t.task_id===failedQuestion.asking_task_id)
  await capture(owner.tui,'review-05-failed-provider')
  await record('review-05-failure',{task:failedTask,question_pending:ended.active_interactions?.some(q=>q.id===failedQuestion.id),provider_pid:active[0].pid})
  results.push({finding:'provider failure visibility',pass:failedTask.state==='blocked'&&failedTask.reason.startsWith('Provider run failed:')&&ended.active_interactions?.some(q=>q.id===failedQuestion.id),state:failedTask.state,reason:failedTask.reason})
  }
  report.review=results
  await record('review-results',results)
  if(process.env.AMASK_EXPECT_REVIEW_RED==='1')assert(results.every(r=>!r.pass),'Every review regression should reproduce before the fixes')
  else assert(results.every(r=>r.pass),'All review regressions must pass')
  report.cells.push({id:'AM-ASK-01',result:process.env.AMASK_EXPECT_REVIEW_RED==='1'?'EXPECTED RED':'REVIEW PASS',review:results})
}
