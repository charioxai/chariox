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
// The drill supplies a fixed command with generated, shell-safe names. Match
// it exactly, allowing only the official harness's shell launch wrapper.
function matchesProbeCommand(actual, expected) {
  if (typeof actual !== "string" || !expected) return false
  if (actual.trim() === expected) return true
  const wrapped = actual.trim().match(/^(?:\/(?:bin|usr\/bin)\/)?(?:bash|sh|zsh) -(?:lc|c) (['"])(.*)\1$/s)
  return wrapped?.[2] === expected
}
export function assertToolProbe(result, expected, label, command) {
  const verified = result.tool_rows?.some(row => {
    let tool
    try { tool = JSON.parse(row) } catch { return false }
    if (tool.status !== "completed" || tool.error || !matchesProbeCommand(tool.input?.command, command)) return false
    const exit = tool.exit_code ?? tool.raw?.match(/(?:^|\n)exit_code:\s*(-?\d+)(?:\n|$)/)?.[1]
    if (exit !== undefined && Number(exit) !== 0) return false
    return typeof tool.output === "string" && tool.output.trim() === expected
  })
  if (result.lifecycle !== "completed" || !verified) {
    throw new Error(`${label} needs a successful tool result for the requested command and output ${expected}`)
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
