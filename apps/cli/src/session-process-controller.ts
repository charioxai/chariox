import type { SessionProcess } from "@chariox/kernel-client/kernel-events"

/// The kernel repeats a quiet process at least every 30 s and sends a changed row at once: a row not heard of for longer is gone.
const PROCESS_ROW_FRESH_MS = 45_000

export function sessionProcessLines(processes: SessionProcess[], now = Date.now(), ownerLabel = (row: SessionProcess) => `${row.owner.kind}:${row.owner.id}`): string[] {
  const columns = (values: string[]) => values.map((value, i) => value.slice(0, [18, 8, 22, 12, 8, 8][i]).padEnd([18, 8, 22, 12, 8, 8][i]!)).join(" ")
  const rows = processes.filter(row => now - row.sampled_at_ms <= PROCESS_ROW_FRESH_MS)
  return [columns(["OWNER", "PID", "COMMAND", "MEMORY", "CPU", "AGE"]), ...rows.flatMap(row => [columns([
    ownerLabel(row), String(row.pid), row.command,
    `${(row.resident_set_bytes / 1048576).toFixed(1)} MiB`,
    `${(row.cpu_millis / 10).toFixed(1)}%`, `${Math.floor((row.age_ms + Math.max(0, now - row.sampled_at_ms)) / 1000)}s`,
  ]), `  ${row.process_id}`]), ...(rows.length ? [] : [processes.length ? "No recent process samples." : "No running processes in this session."])]
}

export function createSessionProcessController(deps: {
  processes: () => SessionProcess[]
  sessionId: () => string | null
  readRoomProcesses: (sessionId: string) => Promise<SessionProcess[]>
  ownerLabel: (row: SessionProcess) => string
  render: () => void
  schedule: (callback: () => void, delay: number) => unknown
  pageSize: () => number
}) {
  let open = false
  let generation = 0
  let offset = 0
  let terminalResources: { sessionId: string; processes: SessionProcess[] | undefined } | null = null
  let confirmation: {agent: string; selected: number; resolve: (value: boolean) => void} | null = null
  const lines = () => {
    const resources = terminalResources?.sessionId === deps.sessionId() ? terminalResources : null
    const processes = deps.processes()
    const rows = resources?.processes
      ? [...processes.filter(row => row.owner.kind !== "terminal"), ...resources.processes]
      : processes
    const visible = sessionProcessLines(rows, Date.now(), deps.ownerLabel)
    if (resources && !resources.processes) visible.push("Terminal processes unavailable.")
    return visible
  }
  const controller = {
    isOpen: () => open,
    confirming: () => confirmation !== null,
    confirmAgentStop(agent: string): Promise<boolean> {
      confirmation?.resolve(false)
      open = true; generation++; offset = 0
      return new Promise(resolve => { confirmation = {agent, selected: 1, resolve}; deps.render() })
    },
    visibleLines: () => { if (confirmation) return [`Stop all processes of ${confirmation.agent}?`, "", `${confirmation.selected === 0 ? ">" : " "} Stop       ${confirmation.selected === 1 ? ">" : " "} Cancel`]; const all = lines(); return [all[0]!, ...all.slice(1 + offset, 1 + offset + Math.max(1, deps.pageSize() - 1))] },
    close() { confirmation?.resolve(false); confirmation = null; open = false; generation++; terminalResources = null; deps.render() },
    open() {
      open = true; offset = 0
      terminalResources = null
      const current = ++generation
      const isCurrent = () => open && current === generation
      const refresh = async () => {
        if (!isCurrent()) return
        const sessionId = deps.sessionId()
        if (!sessionId) return
        let processes: SessionProcess[] | undefined
        try {
          processes = (await deps.readRoomProcesses(sessionId)).filter(row => row.owner.kind === "terminal")
        } catch {
          // Keep the agent samples available when the separate Room resource read fails.
        }
        if (!isCurrent()) return
        if (sessionId === deps.sessionId()) terminalResources = { sessionId, processes }
        deps.render()
        deps.schedule(() => { void refresh() }, 1000)
      }
      deps.render()
      void refresh()
    },
    handleKey(event: { name: string; eventType?: string; ctrl?: boolean }) {
      if (!open) return false
      if (event.eventType === "release") return true
      if (confirmation) {
        if (event.name === "escape") { controller.close(); return true }
        if (["left", "right", "tab"].includes(event.name)) confirmation.selected = 1 - confirmation.selected
        if (event.name === "return" || event.name === "enter") {
          const pending = confirmation; confirmation = null; open = false; generation++; deps.render(); pending.resolve(pending.selected === 0); return true
        }
        deps.render(); return true
      }
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.name === "escape") { controller.close(); return true }
      const amount = event.name === "down" ? 1 : event.name === "up" ? -1 : event.name === "pagedown" ? deps.pageSize() : event.name === "pageup" ? -deps.pageSize() : 0
      offset = Math.max(0, Math.min(Math.max(0, lines().length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  return controller
}
