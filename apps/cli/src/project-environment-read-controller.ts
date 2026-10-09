// MP-08 / MP-10 / MP-11: Detect uses shared request builders.
import { projectEnvironmentOperationRequest, relayStatusRequest } from "@chariox/kernel-client/ipc-requests"
// MP-08 / MP-10: Project read-only navigation, independent of session selection.
import { getProjectEnvironmentRequest, projectEnvironmentLines, type ProjectEnvironment } from "@chariox/kernel-client"

export function projectEnvironmentPageSize(height: number) {
  return Math.max(1, height - Math.max(1, Math.floor(height / 5)) - 4)
}

export function createProjectEnvironmentReadController(deps: {
  send: (request: unknown) => Promise<unknown>
  render: () => void
  pageSize: () => number
}) {
  let open = false
  let generation = 0
  let lines: string[] = []
  let offset = 0
  let current: ProjectEnvironment | null = null
  let detectionGeneration: number | null = null
  const controller = {
    isOpen: () => open,
    visibleLines: () => lines.slice(offset, offset + Math.max(1, deps.pageSize())),
    close() { open = false; generation++; deps.render() },
    async open(projectId: string) {
      const requestGeneration = ++generation
      open = true; current = null; offset = 0; lines = ["Loading Environment…"]; deps.render()
      try {
        const response = await deps.send(getProjectEnvironmentRequest(projectId)) as { ProjectEnvironment?: { environment: ProjectEnvironment } }
        const environment = response.ProjectEnvironment?.environment
        if (!environment || environment.local_project_id !== projectId || environment.schema_version !== 1) throw new Error("Environment unavailable for this Project")
        if (requestGeneration !== generation) return
        current = environment
        lines = viewLines(environment)
      } catch (error) {
        if (requestGeneration !== generation) return
        lines = [error instanceof Error ? error.message : "Environment unavailable"]
      }
      deps.render()
    },
    async detect() {
      if (!current || detectionGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("detect")) return
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
        current = environment; lines = viewLines(environment); offset = 0
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
      offset = Math.max(0, Math.min(Math.max(0, lines.length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  function viewLines(environment: ProjectEnvironment): string[] {
    return [...(environment.delivered_capabilities.enabled_environment_operations.includes("detect") ? ["d · Detect requirements (read-only)", "File names and types from code-manifest folders are sent to your provider.", "Other folders use deterministic detection; model opt-in is available in Web."] : []), ...projectEnvironmentLines(environment)]
  }
  return controller
}
