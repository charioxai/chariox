import type { AgentRuntimeActivity, TerminalOutputRecord } from "../cli-types.js"

// MP-08 / MP-10 / MP-11: retain each completion until its snapshot's output boundary
// reaches this attachment. A newer turn or a busy sibling cannot erase it.
export function createNativeTurnSettlementBarrier() {
  let deliveredWatermark = 0
  const pending = new Map<string, { agentId: string, activity: AgentRuntimeActivity, watermark: number | undefined }>()
  const delivered = new Map<string, string>()
  let latest: Record<string, AgentRuntimeActivity> = {}
  return {
    observe(activity: Record<string, AgentRuntimeActivity>, watermark?: number) {
      latest = activity
      if (watermark !== undefined && watermark < deliveredWatermark) {
        // Reconnect may reach a restarted terminal stream. Old delivered IDs
        // cannot satisfy a completion from its new counter.
        deliveredWatermark = 0
        for (const entry of pending.values()) {
          entry.watermark = activity[entry.agentId]?.last_completed_turn?.prompt_id
            === entry.activity.last_completed_turn?.prompt_id ? watermark : undefined
        }
      }
      for (const [agentId, state] of Object.entries(activity)) {
        const promptId = state.last_completed_turn?.prompt_id
        if (!promptId || delivered.get(agentId) === promptId) continue
        const key = `${agentId}\0${promptId}`
        if (!pending.has(key)) pending.set(key, { agentId, activity: state, watermark })
      }
    },
    projectRecords(records: TerminalOutputRecord[]) {
      for (const record of records) {
        if (typeof record.record_id === "number") deliveredWatermark = Math.max(deliveredWatermark, record.record_id + 1)
      }
    },
    release(outputDrained: boolean): Record<string, AgentRuntimeActivity>[] {
      const ready: Record<string, AgentRuntimeActivity>[] = []
      for (const [key, entry] of pending) {
        if (!outputDrained && (entry.watermark === undefined || deliveredWatermark < entry.watermark)) continue
        ready.push({ [entry.agentId]: { ...(latest[entry.agentId] ?? entry.activity), last_completed_turn: entry.activity.last_completed_turn! } })
        delivered.set(entry.agentId, entry.activity.last_completed_turn!.prompt_id)
        pending.delete(key)
      }
      if (outputDrained && ready.length === 0) ready.push(latest)
      return ready
    },
  }
}
