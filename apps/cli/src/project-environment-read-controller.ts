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
  const controller = {
    isOpen: () => open,
    visibleLines: () => lines.slice(offset, offset + Math.max(1, deps.pageSize())),
    close() { open = false; generation++; deps.render() },
    async open(projectId: string) {
      const requestGeneration = ++generation
      open = true; offset = 0; lines = ["Loading Environment…"]; deps.render()
      try {
        const response = await deps.send(getProjectEnvironmentRequest(projectId)) as { ProjectEnvironment?: { environment: ProjectEnvironment } }
        const environment = response.ProjectEnvironment?.environment
        if (!environment || environment.local_project_id !== projectId || environment.schema_version !== 1) throw new Error("Environment unavailable for this Project")
        if (requestGeneration !== generation) return
        lines = projectEnvironmentLines(environment)
      } catch (error) {
        if (requestGeneration !== generation) return
        lines = [error instanceof Error ? error.message : "Environment unavailable"]
      }
      deps.render()
    },
    handleKey(event: { name: string; eventType?: string; ctrl?: boolean }) {
      if (!open) return false
      if (event.eventType === "release") return true
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.name === "escape") { controller.close(); return true }
      const amount = event.name === "down" ? 1 : event.name === "up" ? -1 : event.name === "pagedown" ? deps.pageSize() : event.name === "pageup" ? -deps.pageSize() : 0
      offset = Math.max(0, Math.min(Math.max(0, lines.length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  return controller
}
