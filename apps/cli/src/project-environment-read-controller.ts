// MP-08 / MP-10 / MP-11: Detect uses shared request builders.
import { projectEnvironmentOperationRequest, relayStatusRequest } from "@chariox/kernel-client/ipc-requests"
// MP-08 / MP-10: Project read-only navigation, independent of session selection.
import { getProjectEnvironmentRequest, projectEnvironmentLines, environmentOriginLabel, environmentRevisionDraft, saveEnvironmentRevisionRequest, type ProjectEnvironment } from "@chariox/kernel-client"

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
  let proposalIndex = 0
  let savingGeneration: number | null = null
  let detectionGeneration: number | null = null
  const controller = {
    isOpen: () => open,
    visibleLines: () => lines.slice(offset, offset + Math.max(1, deps.pageSize())),
    close() { open = false; generation++; deps.render() },
    async open(projectId: string) {
      const requestGeneration = ++generation
      open = true; current = null; proposalIndex = 0; offset = 0; lines = ["Loading Environment…"]; deps.render()
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
    async review(decision: "accept" | "exclude") {
      if (!current || savingGeneration === generation || detectionGeneration === generation || !current.delivered_capabilities.enabled_environment_operations.includes("save")) return
      const proposal = current.proposals[proposalIndex]
      if (!proposal) return
      const requestGeneration = generation
      savingGeneration = requestGeneration
      try {
        const response = await deps.send(saveEnvironmentRevisionRequest(current, environmentRevisionDraft(current), decision === "accept" ? [proposal.proposal_id] : [], decision === "exclude" ? [proposal.proposal_id] : [])) as { ProjectEnvironmentSaved?: { environment: ProjectEnvironment } }
        if (generation !== requestGeneration) return
        const saved = response.ProjectEnvironmentSaved?.environment
        if (!saved || saved.local_project_id !== current.local_project_id) throw new Error("Proposal review unavailable")
        current = saved; proposalIndex = Math.min(proposalIndex,Math.max(0,saved.proposals.length-1)); lines = viewLines(saved); offset = 0
      } catch (error) {
        if (generation === requestGeneration) lines = [error instanceof Error ? error.message : "Proposal review failed", "r · Refresh to review the latest revision", ...viewLines(current)]
      } finally { if (savingGeneration === requestGeneration) savingGeneration = null; if (generation === requestGeneration) deps.render() }
    },
    handleKey(event: { name: string; eventType?: string; ctrl?: boolean; alt?: boolean; meta?: boolean }) {
      if (!open) return false
      if (event.eventType === "release") return true
      if (event.ctrl && (event.name === "c" || event.name === "e")) return false
      if (event.ctrl || event.alt || event.meta) return true
      if (event.name === "r" && current) { void controller.open(current.local_project_id); return true }
      if (event.name === "a" || event.name === "x") { void controller.review(event.name === "a" ? "accept" : "exclude"); return true }
      if ((event.name === "n" || event.name === "p") && current?.proposals.length) {
        proposalIndex = Math.max(0, Math.min(current.proposals.length - 1, proposalIndex + (event.name === "n" ? 1 : -1)))
        lines = viewLines(current); offset = 0; deps.render(); return true
      }
      if (event.name === "d") { void controller.detect(); return true }
      if (event.name === "escape") { controller.close(); return true }
      const amount = event.name === "down" ? 1 : event.name === "up" ? -1 : event.name === "pagedown" ? deps.pageSize() : event.name === "pageup" ? -deps.pageSize() : 0
      offset = Math.max(0, Math.min(Math.max(0, lines.length - deps.pageSize()), offset + amount))
      deps.render()
      return true
    },
  }
  function viewLines(environment: ProjectEnvironment): string[] {
    const selected = environment.proposals[proposalIndex]
    return [`Revision ${environment.revision ?? 0} · r Refresh`, ...(environment.delivered_capabilities.enabled_environment_operations.includes("save") ? ["n/p · Next/previous proposal; a · Accept; x · Exclude", ...(environment.proposals[proposalIndex] ? [`Review ${proposalIndex + 1}/${environment.proposals.length} · ${selected!.requirement.title}`, selected!.requirement.spec.kind === "software" ? `Version constraint: ${selected!.requirement.spec.version_constraint ?? "Not specified"}` : `Kind: ${selected!.requirement.spec.kind}`, ...selected!.requirement.origins.map(environmentOriginLabel), "Field editing is available in the Project’s Web Environment."] : ["No pending proposals"])] : []), ...(environment.delivered_capabilities.enabled_environment_operations.includes("detect") ? ["d · Detect requirements (read-only)", "File names and types from code-manifest folders are sent to your provider.", "Other folders use deterministic detection; model opt-in is available in Web."] : []), ...projectEnvironmentLines(environment)]
  }
  return controller
}
