import type { JobView, SessionProcess, SessionProcessState } from "./kernel-events.js"

/** A command of an agent that waits for room: one queued-job row in its pane. */
export type QueuedJob = {
  processId: string
  command: string
  /** What it waits for, e.g. "waiting for memory, 2nd in line". */
  wait: string
  /** The expected need and what is free, when the kernel knows them. */
  detail: string | null
  remoteAvailable?: boolean
  estimate?: SessionProcess["estimate"]
  rate?: string
  rateDetail?: string
}

type Waiting = Exclude<SessionProcessState, { kind: "running" }>

/** The agent's waiting commands in line order: held commands first by place in
 * line, then paused commands. A row leaves when its command runs. */
export function queuedJobsForAgent(processes: readonly SessionProcess[] | undefined, agentId: string | null | undefined): QueuedJob[] {
  if (!agentId) return []
  const order = (state: Waiting) => [state.kind === "held" ? 0 : 1, state.position]
  return (processes ?? [])
    .filter((row): row is SessionProcess & { state: Waiting } => row.owner.kind === "agent" && row.owner.id === agentId && row.state?.kind !== undefined && row.state.kind !== "running")
    .sort((a, b) => {
      const [x, y] = [order(a.state), order(b.state)]
      return x[0]! - y[0]! || x[1]! - y[1]! || a.pid - b.pid
    })
    .map(row => ({
      processId: row.process_id,
      command: row.state.command_line || row.command,
      wait: `${queuedJobWaitLabel(row.state)}${"remote_refusal" in row && row.remote_refusal === "no_capacity" ? " · no VM free" : ""}`,
      detail: `Needs about ${gigabytes(row.state.need_bytes)}; ${gigabytes(row.state.free_bytes)} free`,
      ...(row.remote_available ? { remoteAvailable: true, ...(row.estimate ? { estimate: row.estimate, ...queuedJobRate(row.estimate) } : {}) } : {}),
    }))
}

export function queuedJobWaitLabel(state: Waiting): string {
  const place = state.position === 1 ? `needs ${gigabytes(state.need_bytes)}` : `${ordinal(Math.max(1, state.position))} in line`
  if (state.kind === "paused" && state.reason === "memory") return "paused at the memory limit"
  return `waiting for ${state.reason === "disk" ? "disk space" : "memory"}, ${place}`
}

export function queuedJobRate(estimate: NonNullable<SessionProcess["estimate"]>): { rate: string; rateDetail: string } {
  const money = (amount: number) => new Intl.NumberFormat("en-US", { style: "currency", currency: estimate.currency, maximumSignificantDigits: 2 }).format(amount)
  const amount = estimate.amount_micros / 1_000_000
  const rate = estimate.kind === "total" && amount < 0.01 ? `< ${money(0.01)}` : money(amount)
  const memory = `${Math.ceil(estimate.memory_mib / 1024)} GB`
  return estimate.kind === "total"
    ? { rate: `≈ ${rate}`, rateDetail: `About ${rate} on a VM (${memory}${estimate.minutes !== undefined ? `, about ${estimate.minutes} min` : ""})` }
    : { rate: `${rate}/min`, rateDetail: `No estimate yet: a ${memory} VM costs ${rate} a minute` }
}

/** Progress and pricing come from the home kernel, including confirmed deletion. */
export function remoteJobLinesForAgent(jobs: readonly JobView[] | undefined, agentId: string | null | undefined, now = Date.now()): string[] {
  return (jobs ?? []).filter(job => job.agent_id === agentId && job.placement === "remote").flatMap(job => {
    const result = job.result, machine = job.machine_label ?? result?.ran_on.machine, address = job.address ?? result?.ran_on.ip
    const allocating = ["starting", "allocating", "queued"].includes(job.state)
    const parts = [allocating ? "Starting a remote machine" : `${result || job.state === "finished" ? "Ran" : "Running"} on ${machine ?? "a remote machine"}`, ...address ? [address] : [], job.command]
    if (!allocating || result) {
      const seconds = Math.floor((result?.duration_ms ?? Math.max(0, now - job.started_at_ms)) / 1000)
      parts.push(`${Math.floor(seconds / 60)}m ${seconds % 60}s`)
    }
    if (result) parts.push(result.exit_code !== null ? `exit ${result.exit_code}` : result.outcome.replaceAll("_", " "), `${result.files_back.length} ${result.files_back.length === 1 ? "file" : "files"} back`, ...result.conflicts.length ? [`${result.conflicts.length} conflicts`] : [])
    if (job.class_id) parts.push(job.class_id)
    if (job.rate_per_minute) {
      const estimate = { ...job.rate_per_minute, kind: "per_minute" as const, memory_mib: 0 }
      parts.push(queuedJobRate(estimate).rate)
      if (job.estimate_micros !== null && job.estimate_micros !== undefined) parts.push(queuedJobRate({ ...estimate, kind: "total", amount_micros: job.estimate_micros }).rate)
    }
    return [parts.join(" · "), ...(job.deleted ? [`Deleted ${machine ?? "remote machine"}`] : [])]
  })
}

function gigabytes(bytes: number): string {
  return `${Math.max(1, Math.round(bytes / 2 ** 30))} GB`
}

function ordinal(n: number): string {
  const tens = n % 100
  const suffix = tens >= 11 && tens <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" } as Record<number, string>)[n % 10] ?? "th"
  return `${n}${suffix}`
}
