export function evidenceName(from, to, stamp) {
  const safe = value => String(value).replace(/[^a-zA-Z0-9._-]+/g, "-")
  return `${safe(from.provider)}-${safe(from.model)}-to-${safe(to.provider)}-${safe(to.model)}-${stamp}.json`
}
export function scoreFacts(text, facts) {
  const line = text.split("\n").find(line => line.includes("=")) ?? ""
  const answers = Object.fromEntries([...line.matchAll(/([a-z_]+)=(.*?)(?=\s+[a-z_]+=|$)/g)].map(([, key, value]) => [key, value.toLowerCase()]))
  return Object.fromEntries(Object.entries(facts).map(([key, value]) => [key, (answers[key] ?? "").split(/[\s,;`'"]+/).map(token => token.replace(/\.$/, "")).includes(String(value).toLowerCase())]))
}
export function scoreSummary(recalled) {
  return { correct: Object.values(recalled).filter(Boolean).length, total: Object.keys(recalled).length }
}
export function assertToolProbe(result, expected, label) {
  if (result.lifecycle !== "completed" || !result.tool_rows?.length || !result.text.includes(expected)) {
    throw new Error(`${label} needs a completed tool call and verified output ${expected}`)
  }
}

export function agentSnapshot(agent) {
  const remote = agent.remote_execution
  return { provider: agent.provider, model: agent.model, effort: agent.effort, account_profile: agent.account_profile,
    remote_execution: remote && Object.fromEntries(["worker_kernel_id", "worker_machine_id", "execution_lease_id", "leased_agent_id"].map(key => [key, remote[key]])) }
}
export function assertContinuedPlacement(source, target, placement) {
  if (!placement.kernelRef && !placement.sliceRef) return
  const before = source.remote_execution, after = target.remote_execution
  if (!before || !after || ["worker_kernel_id", "execution_lease_id", "leased_agent_id"].some(key => !before[key] || before[key] !== after[key])) {
    throw new Error("profile switch changed or lost the home-owned worker binding")
  }
}
