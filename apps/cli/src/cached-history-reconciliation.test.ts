import assert from "node:assert/strict"
import test from "node:test"
import {reconcileCachedHistory} from "./cached-history-reconciliation.js"
import type {RuntimeSession} from "./cli-types.js"
import type {CliCachedView} from "./client-view-cache.js"
const entry=(index:number,text:string)=>({id:index,role:"user" as const,text,historyEntryIndex:index,turnId:index})
const session={id:"session",agents:[{id:"a"}]} as RuntimeSession
const cached:CliCachedView={scope:{owner:"user",kernel:"kernel"},sessionId:"session",agentId:"a",alias:"review",sequence:30,savedAt:1,entries:[entry(10,"first"),entry(20,"second"),entry(30,"stale latest")]}
const history={sessionId:"session",visibleAgentId:"a",agentEntries:{a:[entry(30,"current latest")]},historyEntries:[entry(30,"current latest")],promptHistoryEntries:[],nextHistoryCursor:null}
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
