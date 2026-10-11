// MP-08 / MP-10 / MP-11: TUI drives the same kernel plan and durable recovery operations.
import type { ProjectEnvironment, EnvironmentApplyPlan, EnvironmentOperation } from "@chariox/kernel-client"
import { getProjectEnvironmentRequest } from "@chariox/kernel-client"
const active = (o: EnvironmentOperation | null) => !!o && ["requested","preparing","running","validating"].includes(o.phase)
export function createEnvironmentApplyController(deps: {
  send: (request: unknown) => Promise<unknown>; current: () => ProjectEnvironment | null;
  snapshot: (environment: ProjectEnvironment) => void; render: () => void
}) {
  let plan: EnvironmentApplyPlan | null = null, operation: EnvironmentOperation | null = null
  let optIndex = 0, accepted: string[] = [], sending = false, error: string | null = null, generation = 0
  let pendingApply: unknown | null = null
  const watches = new Map<string, Promise<void>>()
  function currentOperation() {
    const env = deps.current()
    if (operation?.local_project_id !== env?.local_project_id) operation = null
    const latest = [...(env?.operations ?? [])].filter(o => o.kind === "apply").sort((a,b) => b.updated_at_ms-a.updated_at_ms || b.attempt-a.attempt)[0]
    if (latest && (!operation || latest.updated_at_ms > operation.updated_at_ms || (latest.operation_id === operation.operation_id && latest.attempt > operation.attempt))) operation = latest
    return operation
  }
  async function poll(op: EnvironmentOperation, g: number) {
    for (let i=0; i<1200 && g===generation; i++) {
      const value = await deps.send({GetEnvironmentOperation:{projectId:op.local_project_id,operationId:op.operation_id}}) as {EnvironmentOperation?:{operation:EnvironmentOperation}}
      if (g !== generation) return
      const next=value.EnvironmentOperation?.operation
      if (!next || next.local_project_id!==op.local_project_id || next.operation_id!==op.operation_id) throw new Error("Apply status unavailable; refresh to recover")
      operation=next; deps.render()
      if(!active(next)) {
        const response=await deps.send(getProjectEnvironmentRequest(op.local_project_id)) as {ProjectEnvironment?:{environment:ProjectEnvironment}}
        if(g===generation && response.ProjectEnvironment?.environment.local_project_id===op.local_project_id)deps.snapshot(response.ProjectEnvironment.environment)
        return
      }
      await new Promise(resolve=>setTimeout(resolve,250))
    }
  }
  function watch(op: EnvironmentOperation, g: number) {
    const key=`${g}:${op.operation_id}`
    const pending=watches.get(key)
    if(pending)return pending
    const task=poll(op,g).finally(()=>{if(watches.get(key)===task)watches.delete(key)})
    watches.set(key,task)
    return task
  }
  async function send(kind: "plan" | "apply" | "cancel" | "retry") {
    const env=deps.current(); if(!env || sending)return
    const g=generation; sending=true; error=null; deps.render()
    try {
      let payload:unknown
      if(kind==="plan") {
        if(pendingApply)throw new Error("Recover the original Apply acknowledgement first")
        const relay=await deps.send({RelayStatus:null}) as {RelayStatus?:{status:{machine_id:string;daemon_id:string}}}
        if(g!==generation)return
        const target=relay.RelayStatus?.status;if(!target)throw new Error("Owning kernel identity unavailable")
        payload={PlanProjectEnvironment:{projectId:env.local_project_id,expectedRevision:env.revision,revisionDigest:env.content_digest,target:{machine_id:target.machine_id,target_instance_generation:target.daemon_id,slice_ref:null},selectedItems:[]}}
      } else if(kind==="apply" && plan && plan.revision_digest===env.content_digest) {
        payload=pendingApply ?? {ApplyProjectEnvironment:{projectId:env.local_project_id,operationId:`apply-${crypto.randomUUID()}`,planId:plan.plan_id,expectedRevision:env.revision,revisionDigest:env.content_digest,target:plan.target,selectedItems:plan.selected_items,perItemOptIns:plan.required_opt_ins.map(id=>({requirement_id:id,decision:accepted.includes(id)?"accepted":"excluded",recorded_at_ms:Date.now(),user_id:env.reviewed_by}))}}
        pendingApply=payload
      } else {
        if(kind==="apply")throw new Error("Plan the current revision before Apply")
        const op=currentOperation();if(!op)throw new Error("Apply operation unavailable")
        payload=kind==="cancel"?{CancelEnvironmentOperation:{projectId:env.local_project_id,operationId:op.operation_id}}:{RetryEnvironmentOperation:{projectId:env.local_project_id,operationId:op.operation_id,expectedAttempt:op.attempt}}
      }
      const response=await deps.send(payload) as {ProjectEnvironmentPlan?:{plan:EnvironmentApplyPlan};EnvironmentOperation?:{operation:EnvironmentOperation}}
      if(g!==generation)return
      if(kind==="plan") {
        const next=response.ProjectEnvironmentPlan?.plan
        if(!next || next.project_id!==env.local_project_id || next.revision_digest!==env.content_digest)throw new Error("Plan unavailable for this revision")
        plan=next;accepted=[];optIndex=0
      } else {
        const next=response.EnvironmentOperation?.operation
        if(!next || next.local_project_id!==env.local_project_id)throw new Error("Apply unavailable")
        if(kind==="apply")pendingApply=null
        operation=next
        // Cancel remains usable while the acknowledged operation is running.
        sending=false;deps.render()
        await watch(next,g)
      }
    } catch(cause){if(g===generation)error=cause instanceof Error?cause.message:"Apply unavailable"}
    finally{if(g===generation){sending=false;deps.render()}}
  }
  return {
    reset(){generation++;watches.clear();pendingApply=null;plan=null;operation=null;accepted=[];sending=false;error=null},
    busy:()=>sending || !!pendingApply || active(currentOperation()),
    lines(){
      const env=deps.current();if(!env?.delivered_capabilities.enabled_environment_operations.includes("apply"))return []
      if(plan?.revision_digest!==env.content_digest)plan=null
      const op=currentOperation()
      // Opening/reconnecting to another client's acknowledged attempt resumes
      // observation, without creating an operation or a second cancel watch.
      if(op && active(op) && !sending && !error){const g=generation;void watch(op,g).catch(cause=>{if(g===generation){error=cause instanceof Error?cause.message:"Apply status unavailable";deps.render()}})}
      return ["s · Set up this machine / preview; k · Cancel Apply; u · Recheck and retry",...(plan?[`Plan · ${plan.target.machine_id} · generation ${plan.target.target_instance_generation}`,pendingApply?"Enter · Recover Apply acknowledgement":"Enter · Apply reviewed plan; n/p · Setup item; Space · Opt in",...plan.conflicts.map(r=>`${r.requirement_id} · Conflict · ${r.safe_summary}`),...plan.required_opt_ins.map((id,i)=>`${i===optIndex?">":" "} [${accepted.includes(id)?"x":" "}] Allow bounded Setup · ${id}`)]:[]),...(op?[`Apply · ${op.phase} · attempt ${op.attempt} · ${op.operation_id}`,...op.per_item_results.map(r=>`${r.requirement_id} · ${r.status} · ${r.safe_summary} · receipts ${r.receipt_ids.length}`)]:[]),...(error?[error]:[])]
    },
    handleKey(name:string){
      const env=deps.current();if(!env?.delivered_capabilities.enabled_environment_operations.includes("apply"))return false
      if(name==="s" && !pendingApply && !sending && !active(currentOperation())){void send("plan");return true}
      if(name==="k" && active(currentOperation())){void send("cancel");return true}
      if(name==="u" && ["failed","cancelled","awaiting_input"].includes(currentOperation()?.phase??"")){void send("retry");return true}
      if(!plan || plan.revision_digest!==env.content_digest || active(currentOperation()))return false
      if(name==="return" || name==="enter"){void send("apply");return true}
      if(pendingApply)return false
      if(name==="n" || name==="p"){optIndex=Math.max(0,Math.min(plan.required_opt_ins.length-1,optIndex+(name==="n"?1:-1)));deps.render();return true}
      if(name==="space"){const id=plan.required_opt_ins[optIndex];if(id)accepted=accepted.includes(id)?accepted.filter(v=>v!==id):[...accepted,id];deps.render();return true}
      return false
    },
  }
}
