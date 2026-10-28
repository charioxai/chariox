// MP-08 / MP-10 / MP-11: Detect uses shared request builders.
import { projectEnvironmentOperationRequest, relayStatusRequest } from "@chariox/kernel-client/ipc-requests"
// MP-08 / MP-10: Project read-only navigation, independent of session selection.
import { environmentReadFailureMessage } from "./project-environment-read-errors.js"
import { getProjectEnvironmentRequest, projectEnvironmentLines, type ProjectEnvironment } from "@chariox/kernel-client"
import { createEnvironmentViewCache } from "@chariox/kernel-client"

// MP-08 / MP-10 / MP-11: show only actions this release really supports.
const environmentReadIntroduction = [
  "An Environment lists what this project needs on this machine.",
  "Read only: this release can show requirements, but cannot check or change them here.",
  "After the next release, you can add requirements and check this machine here.",
  "",
]

export function projectEnvironmentPageSize(height: number) {
  return Math.max(1, height - Math.max(1, Math.floor(height / 5)) - 4)
}

// Inner text width of the Environment panel (see cli-dialog-overlay.ts: panel width, 2-column padding each side).
export function projectEnvironmentPanelWidth(viewportWidth: number) {
  return Math.min(110, Math.max(30, viewportWidth - 4))
}

export function projectEnvironmentTextWidth(viewportWidth: number) {
  return Math.max(1, Math.min(projectEnvironmentPanelWidth(viewportWidth), viewportWidth - 4) - 4)
}

// Word-wrap keeping the line's indentation; words longer than the row are split.
function wrapLine(line: string, width: number): string[] {
  if (line.length <= width) return [line]
  const indent = " ".repeat(Math.min(line.length - line.trimStart().length, Math.floor(width / 2)))
  const room = width - indent.length
  const words = line.trimStart().split(" ").flatMap(word => word.match(new RegExp(`.{1,${room}}`, "gs")) ?? [""])
  const rows: string[] = []
  for (const word of words) {
    const last = rows.at(-1)
    if (last !== undefined && last.length + 1 + word.length <= room) rows[rows.length - 1] = `${last} ${word}`
    else rows.push(word)
  }
  return rows.map(row => indent + row)
}

export function createProjectEnvironmentReadController(deps: {
  send: (request: unknown) => Promise<unknown>
  render: () => void
  pageSize: () => number
  textWidth: () => number
}) {
  const cache = createEnvironmentViewCache()
  const scope = Symbol("kernel-client-view")
  let refreshing = false
  let open = false
  let generation = 0
  let lines: string[] = []
  let offset = 0
  const displayLines = () => lines.flatMap(line => wrapLine(line, deps.textWidth()))
  let current: ProjectEnvironment | null = null
  let detectionGeneration: number | null = null
  const controller = {
    isOpen: () => open,
    visibleLines: () => displayLines().slice(offset, offset + Math.max(1, deps.pageSize())),
    close() { open = false; generation++; deps.render() },
    async open(projectId: string) {
      const requestGeneration = ++generation
      open = true; refreshing = true; current = cache.peek(scope, projectId); offset = 0
      lines = current ? ["Refreshing Environment… · last kernel snapshot", ...viewLines(current)] : ["Loading Environment…"]; deps.render()
      try {
        const response = await deps.send(getProjectEnvironmentRequest(projectId)) as { ProjectEnvironment?: { environment: ProjectEnvironment } }
        const environment = response.ProjectEnvironment?.environment
        if (!environment || environment.local_project_id !== projectId || environment.schema_version !== 1) throw new Error("Environment unavailable for this Project")
        if (requestGeneration !== generation) return
        cache.remember(scope, environment); current = environment
        lines = [...environmentReadIntroduction, ...viewLines(environment)]
      } catch (error) {
        if (requestGeneration !== generation) return
        cache.forget(scope, projectId); current = null
        lines = [...environmentReadIntroduction, environmentReadFailureMessage(error)]
      }
      if (requestGeneration === generation) refreshing = false
      deps.render()
    },
    async detect() {
      if (!current || refreshing || detectionGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("detect")) return
      const requestGeneration = generation
      const projectId = current.local_project_id
      detectionGeneration = requestGeneration
      lines = ["Detecting requirements…"]; deps.render()
      try {
        const statusResponse = await deps.send(relayStatusRequest()) as { RelayStatus?: { status: { machine_id: string; daemon_id: string } } }
        const status = statusResponse.RelayStatus?.status
        if (!status?.machine_id || !status.daemon_id) throw new Error("Kernel identity unavailable")
        if (requestGeneration !== generation) return
        const response = await deps.send(projectEnvironmentOperationRequest("DetectProjectEnvironment", { projectId, operationId: `detect-${crypto.randomUUID()}`, folderIds: [], target: { machine_id: status.machine_id, target_instance_generation: status.daemon_id, slice_ref: null }, provider: null, allowModelFolders: [] })) as { ProjectEnvironment?: { environment: ProjectEnvironment } }
        const environment = response.ProjectEnvironment?.environment
        if (!environment || environment.local_project_id !== projectId) throw new Error("Detect unavailable for this Project")
        if (requestGeneration !== generation) return
        cache.remember(scope, environment); current = environment; lines = viewLines(environment); offset = 0
      } catch (error) {
        if (requestGeneration !== generation) return
        lines = [error instanceof Error ? error.message : "Detect unavailable", ...viewLines(current!)]
      } finally { if (detectionGeneration === requestGeneration) detectionGeneration = null; if (requestGeneration === generation) deps.render() }
    },
    handleKey(event: { name: string; eventType?: string; ctrl?: boolean }) {
      if (!open) return false
      if (event.eventType === "release") return true
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.name === "d") { void controller.detect(); return true }
      if (event.name === "escape") { controller.close(); return true }
      const amount = event.name === "down" ? 1 : event.name === "up" ? -1 : event.name === "pagedown" ? deps.pageSize() : event.name === "pageup" ? -deps.pageSize() : 0
      offset = Math.max(0, Math.min(Math.max(0, displayLines().length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  function viewLines(environment: ProjectEnvironment): string[] {
    return [...(environment.delivered_capabilities.enabled_environment_operations.includes("detect") ? ["d · Detect requirements (read-only)", "File names and types from code-manifest folders are sent to your provider.", "Other folders use deterministic detection; model opt-in is available in Web."] : []), ...projectEnvironmentLines(environment)]
  }
  return controller
}
