// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: real TUI drill exposed a missing render binding.
import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcClient } from "./ipc.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import type { RuntimeSession } from "./cli-types.js"

test("goal admission applies the home revision and repaints through the app render binding", async () => {
  let state = { id: "room", agents: [{ id: "agent", alias: "worker" }] } as RuntimeSession
  const admitted = { ...state, agent_tasks: [{ task_id: "prompt", revision: 2 }] } as RuntimeSession
  const calls: string[] = []
  const client = new LocalIpcClient("/unused-goal-test.sock")
  client.send = async <TResponse>(request: unknown): Promise<TResponse> => {
    assert.deepEqual(request, { SubmitPrompt: { session_id: "room", attachment_id: "terminal", target_agent_id: "agent", prompt: "/goal Implement real work", attachments: [] } })
    return { PromptSubmitted: { session: admitted } } as TResponse
  }
  const handlers = createCliCommandActionComposition({
    client, options: {}, preferencesState: () => ({}), isAttached: () => true,
    sessionState: () => state, attachmentState: () => ({ id: "terminal" }), focusedAgentId: () => "agent",
    applySessionState: (next: RuntimeSession) => { state = next; calls.push("apply") },
    refreshAgentPanes: async (next: RuntimeSession) => { assert.equal(next, admitted); calls.push("panes") },
    rebuildTranscript: () => calls.push("transcript"), requestRootRender: () => calls.push("render"),
    flashFooter: (message: string) => assert.fail(message),
  } as unknown as Parameters<typeof createCliCommandActionComposition>[0])
  await handlers.handleGoalCommand({ kind: "goal", raw: "/goal Implement real work", prompt: "Implement real work" })
  assert.equal(state, admitted)
  assert.deepEqual(calls, ["apply", "panes", "transcript", "render"])
})

// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: compact controls preserve unrelated room history.
test("goal control applies its compact revision without rebuilding transcript", async () => {
  let state={id:"room",agents:[{id:"agent",alias:"worker"}],agent_tasks:[{task_id:"prompt",agent_id:"agent",revision:7,state:"working",goal:{id:"goal:prompt"}}]} as RuntimeSession
  const updated={...state.agent_tasks![0]!,room_id:"room",revision:8,state:"blocked",goal:{...state.agent_tasks![0]!.goal!,paused:true}}
  const calls:string[]=[]
  const client=new LocalIpcClient("/unused-goal-test.sock")
  client.send=async <TResponse>():Promise<TResponse>=>({AgentTaskUpdated:{session_id:"room",task:updated}} as TResponse)
  const handlers=createCliCommandActionComposition({client,options:{},preferencesState:()=>({}),isAttached:()=>true,sessionState:()=>state,attachmentState:()=>({id:"terminal"}),focusedAgentId:()=>"agent",applySessionState:(next:RuntimeSession)=>{state=next;calls.push("apply")},refreshAgentPanes:async()=>assert.fail("compact control must not rebuild unrelated panes"),rebuildTranscript:()=>assert.fail("compact control must not rebuild transcript"),requestRootRender:()=>calls.push("render"),flashFooter:(message:string)=>assert.fail(message)} as unknown as Parameters<typeof createCliCommandActionComposition>[0])
  await handlers.handleGoalCommand({kind:"goal",raw:"/goal pause",prompt:"pause"})
  assert.equal(state.agent_tasks![0]!.revision,8)
  assert.deepEqual(calls,["apply","render"])
})
