import assert from "node:assert/strict"
import test from "node:test"
import {reconcileCachedHistory} from "./cached-history-reconciliation.js"
import type {RuntimeSession} from "./cli-types.js"
import type {CliCachedView} from "./client-view-cache.js"
const entry=(index:number,text:string)=>({id:index,role:"user" as const,text,historyEntryIndex:index,turnId:index})
const session={id:"session",agents:[{id:"a"}]} as RuntimeSession
const cached:CliCachedView={scope:{owner:"user",kernel:"kernel"},sessionId:"session",agentId:"a",alias:"review",sequence:30,savedAt:1,entries:[entry(10,"first"),entry(20,"second"),entry(30,"stale latest")]}
const history={sessionId:"session",visibleAgentId:"a",agentEntries:{a:[entry(30,"current latest")]},historyEntries:[entry(30,"current latest")],promptHistoryEntries:[],nextHistoryCursor:{agentId:"a",cursor:{before_sequence:30}}}
test("MP-08 / MP-10 / MP-11 latest-turn handoff preserves earlier cached rows exactly once and lets the kernel replace its overlap",()=>{
 const result=reconcileCachedHistory(cached,session,history)
 assert.deepEqual(result.history.historyEntries.map(entry=>entry.text),["first","second","current latest"])
 assert.equal(result.discard,false)
 assert.equal(new Set(result.history.historyEntries.map(entry=>entry.id)).size,3)
 assert.deepEqual(history.historyEntries.map(entry=>entry.text),["current latest"])
})
test("MP-08 / MP-10 / MP-11 reset sequences and invisible agents never seed cached rows",()=>{
 assert.equal(reconcileCachedHistory({...cached,sequence:31},session,history).discard,true)
 assert.equal(reconcileCachedHistory(cached,{...session,agents:[]},history).history,history)
 assert.equal(reconcileCachedHistory({...cached,sessionId:"other"},session,history).history,history)
})

test("MP-08 / MP-10 / MP-11 paging resumes before the restored prefix rather than downloading it again",()=>{
 const paged={...history,nextHistoryCursor:{agentId:"a",cursor:{before_sequence:30}}}
 const result=reconcileCachedHistory(cached,session,paged)
 assert.deepEqual(result.history.nextHistoryCursor,{agentId:"a",cursor:{before_sequence:10}})
 assert.deepEqual(paged.nextHistoryCursor,{agentId:"a",cursor:{before_sequence:30}})
})

test("MP-08 / MP-10 / MP-11 a server with no earlier history wins over a cached prefix",()=>{
 const complete={...history,nextHistoryCursor:null}
 assert.equal(reconcileCachedHistory(cached,session,complete).history,complete)
})
test("MP-08 / MP-10 / MP-11 restored historical rows cannot restore queued prompt controls",()=>{
 const old={...cached,entries:cached.entries.map(entry=>({...entry,queuedPrompt:{promptId:"retired"}}))} as unknown as CliCachedView
 const restored=reconcileCachedHistory(old,session,history).history.historyEntries
 assert.equal(restored.length,3);assert.ok(restored.every(entry=>entry.queuedPrompt===undefined))
})

// MP-08 / MP-10 / MP-11: a cached prefix cannot skip turns produced while the client was closed.
test("cached history leaves the server cursor intact when an unseen turn separates it from the latest turn", () => {
 const current = [entry(50, "fifth")]
 const gap = {...history,agentEntries:{a:current},historyEntries:current,nextHistoryCursor:{agentId:"a",cursor:{before_sequence:50}}}
 const result = reconcileCachedHistory(cached,session,gap)
 assert.equal(result.history,gap)
 assert.deepEqual(result.history.nextHistoryCursor,{agentId:"a",cursor:{before_sequence:50}})
 // Paging before 50 can still retrieve the unseen turn at 40.
 assert.equal(result.discard,false)
})
