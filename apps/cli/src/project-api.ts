import type { RuntimeProject } from "@arroba/kernel-client"

import type { LocalIpcClient } from "./ipc.js"
import { expectVariant } from "./ipc-response.js"

type ProjectMutationResult = {
  project: RuntimeProject
  sessions?: Array<{ id: string; alias?: string | null }>
}

export async function listProjects(
  client: LocalIpcClient,
  includeArchived = true,
): Promise<RuntimeProject[]> {
  const response = await client.send<Record<string, unknown>>({
    ListProjects: { include_archived: includeArchived },
  })
  return expectVariant<{ projects: RuntimeProject[] }>(response, "ProjectsListed").projects
}

export async function renameProject(
  client: LocalIpcClient,
  projectId: string,
  name: string,
): Promise<RuntimeProject> {
  const response = await client.send<Record<string, unknown>>({
    RenameProject: { project_id: projectId, name },
  })
  return expectVariant<{ project: RuntimeProject }>(response, "ProjectRenamed").project
}

export async function archiveProject(
  client: LocalIpcClient,
  projectId: string,
): Promise<ProjectMutationResult> {
  const response = await client.send<Record<string, unknown>>({
    ArchiveProject: { project_id: projectId },
  })
  return expectVariant<ProjectMutationResult>(response, "ProjectArchived")
}

export async function deleteProject(
  client: LocalIpcClient,
  projectId: string,
): Promise<ProjectMutationResult> {
  const response = await client.send<Record<string, unknown>>({
    DeleteProject: { project_id: projectId },
  })
  return expectVariant<ProjectMutationResult>(response, "ProjectDeleted")
}

export async function restoreProject(
  client: LocalIpcClient,
  projectId: string,
): Promise<RuntimeProject> {
  const response = await client.send<Record<string, unknown>>({
    RestoreProject: { project_id: projectId },
  })
  return expectVariant<{ project: RuntimeProject }>(response, "ProjectRestored").project
}
