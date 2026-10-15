// MP-08 / MP-10 / MP-11: recovery controls use the acknowledged kernel identity.
import test from "node:test"
import assert from "node:assert/strict"
import { createEnvironmentApplyController } from "./project-environment-apply-controller.js"
import type { ProjectEnvironment, EnvironmentApplyPlan, EnvironmentOperation } from "@chariox/kernel-client"
const env = { local_project_id:"project",revision:3,content_digest:"revision",reviewed_by:"owner",operations:[],delivered_capabilities:{enabled_environment_operations:["plan","apply","cancel_operation","retry_operation"]} } as unknown as ProjectEnvironment
const plan: EnvironmentApplyPlan = { plan_id:"plan",project_id:"project",revision_digest:"revision",target:{machine_id:"machine",target_instance_generation:"process-generation",slice_ref:null},selected_items:["public","setup"],required_opt_ins:["setup"],conflicts:[],expires_at_ms:Date.now()+300000 }
test("MP-08 MP-10 MP-11 TUI applies the measured plan target and explicit Setup opt-in",async()=>{
  const requests:unknown[]=[]
  let snapshot=env
  let operation:EnvironmentOperation
  const controller=createEnvironmentApplyController({current:()=>snapshot,snapshot:value=>{snapshot=value},render:()=>{},send:async request=>{
    requests.push(request)
    if("RelayStatus" in (request as object))return {RelayStatus:{status:{machine_id:"machine",daemon_id:"durable-kernel"}}}
    if("PlanProjectEnvironment" in (request as object))return {ProjectEnvironmentPlan:{plan}}
    if("ApplyProjectEnvironment" in (request as object)){
      const payload=(request as {ApplyProjectEnvironment:{operationId:string}}).ApplyProjectEnvironment
      operation={operation_id:payload.operationId,local_project_id:"project",phase:"ready",kind:"apply",attempt:1,updated_at_ms:1,per_item_results:[],receipts:[]} as unknown as EnvironmentOperation
      return {EnvironmentOperation:{operation}}
    }
    if("GetEnvironmentOperation" in (request as object))return {EnvironmentOperation:{operation}}
    return {ProjectEnvironment:{environment:env}}
  }})
  assert.equal(controller.handleKey("s"),true)
  await new Promise(resolve=>setTimeout(resolve,0))
  assert.equal(controller.handleKey("space"),true)
  assert.equal(controller.handleKey("return"),true)
  await new Promise(resolve=>setTimeout(resolve,0))
  const apply=(requests.find(r=>"ApplyProjectEnvironment" in (r as object)) as {ApplyProjectEnvironment:{target:unknown;selectedItems:unknown;perItemOptIns:{decision:string;user_id:string}[]}}).ApplyProjectEnvironment
  assert.deepEqual(apply.target,plan.target)
  assert.deepEqual(apply.selectedItems,plan.selected_items)
  assert.equal(apply.perItemOptIns[0]!.decision,"accepted")
  assert.equal(apply.perItemOptIns[0]!.user_id,"owner")
  controller.reset()
})
test("MP-08 MP-10 MP-11 changing Project discards a late plan acknowledgement",async()=>{
  let resolve:(value:unknown)=>void=()=>{}
  const controller=createEnvironmentApplyController({current:()=>env,snapshot:()=>{},render:()=>{},send:async request=>"RelayStatus" in (request as object)?{RelayStatus:{status:{machine_id:"machine",daemon_id:"kernel"}}}:new Promise(r=>{resolve=r})})
  controller.handleKey("s");await new Promise(r=>setTimeout(r,0));controller.reset();resolve({ProjectEnvironmentPlan:{plan}})
  await new Promise(r=>setTimeout(r,0))
  assert.equal(controller.handleKey("return"),false)
  assert.ok(!controller.lines().some(line=>line.startsWith("Plan ·")))
})
test("MP-08 MP-10 MP-11 reopening an active Apply resumes one status watch",async()=>{
  const running={operation_id:"other-client",local_project_id:"project",phase:"running",kind:"apply",attempt:1,updated_at_ms:1,per_item_results:[],receipts:[]} as unknown as EnvironmentOperation
  let snapshot: ProjectEnvironment={...env,operations:[running]}
  let reads=0
  const controller=createEnvironmentApplyController({current:()=>snapshot,snapshot:value=>{snapshot=value},render:()=>{},send:async request=>{
    if("GetEnvironmentOperation" in (request as object)){reads++;return {EnvironmentOperation:{operation:{...running,phase:"ready",updated_at_ms:2}}}}
    return {ProjectEnvironment:{environment:{...env,operations:[{...running,phase:"ready",updated_at_ms:2}]}}}
  }})
  controller.lines(); controller.lines()
  await new Promise(r=>setTimeout(r,0))
  assert.equal(reads,1,"an acknowledged active snapshot resumes a single watch")
  assert.equal(controller.busy(),false)
  controller.reset()
})
test("MP-08 MP-10 MP-11 a lost Apply ACK keeps its original request",async()=>{
  const payloads:unknown[]=[]
  let operation:EnvironmentOperation
  const controller=createEnvironmentApplyController({current:()=>env,snapshot:()=>{},render:()=>{},send:async request=>{
    if("RelayStatus" in (request as object))return {RelayStatus:{status:{machine_id:"machine",daemon_id:"kernel"}}}
    if("PlanProjectEnvironment" in (request as object))return {ProjectEnvironmentPlan:{plan}}
    if("ApplyProjectEnvironment" in (request as object)){
      payloads.push(request);if(payloads.length===1)throw new Error("ACK lost")
      const id=(request as {ApplyProjectEnvironment:{operationId:string}}).ApplyProjectEnvironment.operationId
      operation={operation_id:id,local_project_id:"project",phase:"ready",kind:"apply",attempt:1,updated_at_ms:1,per_item_results:[],receipts:[]} as unknown as EnvironmentOperation
      return {EnvironmentOperation:{operation}}
    }
    if("GetEnvironmentOperation" in (request as object))return {EnvironmentOperation:{operation}}
    return {ProjectEnvironment:{environment:env}}
  }})
  controller.handleKey("s");await new Promise(r=>setTimeout(r,0))
  controller.handleKey("return");await new Promise(r=>setTimeout(r,0))
  controller.handleKey("return");await new Promise(r=>setTimeout(r,0))
  assert.equal(payloads.length,2)
  assert.deepEqual(payloads[1],payloads[0])
  controller.reset()
})
