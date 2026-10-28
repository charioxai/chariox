// MP-08 / MP-10 / MP-11: standalone entry uses the selected kernel's Project authority.
import type { RuntimeProject } from "./kernel-types.js"
export const projectResolutionMinimumProtocolVersion = 661
export function resolveProjectRequest(workspaceId: string, projectId: string | null = null, createIfMissing = false) {
  return { ResolveProject: { workspace_id: workspaceId, project_id: projectId, create_if_missing: createIfMissing } }
}
export function projectResolutionFromResponse(response: unknown): RuntimeProject | null {
  const value = response as { ProjectResolved?: { project?: RuntimeProject | null } } | null
  const project = value?.ProjectResolved?.project
  if (project === null) return null
  if (!project || typeof project.id !== "string" || !project.id || typeof project.name !== "string" || typeof project.workspace_id !== "string") throw new Error("Project setup response is unavailable. Reopen Environment and try again.")
  return project
}

export function projectResolutionFailureMessage(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : ""
  if (/requires kernel protocol|kernel too old|kernel support unknown|unknown variant.*ResolveProject/i.test(message)) return "Upgrade this kernel to set up Project Environments."
  if (message.includes("Kernel selection changed")) return "Kernel selection changed. Close Environment and reopen it on the selected kernel."
  if (message.includes("archived")) return "This Project is archived. Restore it before setting up its Environment."
  if (message.includes("caller does not own")) return "Select a Project and folder that belong to your account on this kernel."
  if (message.includes("protected Chariox")) return "Choose a project folder outside Chariox’s private state."
  if (message.includes("working directory")) return "This folder is unavailable on the selected kernel. Choose an existing folder and reopen Environment."
  return "Environment setup unavailable. Check the kernel connection and try again."
}
