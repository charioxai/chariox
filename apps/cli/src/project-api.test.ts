import assert from "node:assert/strict"
import test from "node:test"

import {
  archiveProject,
  deleteProject,
  listProjects,
  renameProject,
  restoreProject,
  updateProjectWorkspaces,
} from "./project-api.js"
import type { LocalIpcClient } from "./ipc.js"

test("project API sends the kernel-owned project lifecycle variants", async () => {
  const requests: unknown[] = []
  const project = runtimeProject()
  const responses = [
    { ProjectsListed: { projects: [project] } },
    { ProjectRenamed: { project: { ...project, name: "Renamed" } } },
    { ProjectWorkspacesUpdated: { project: { ...project, workspace_ids: ["/repo", "/shared"] } } },
    { ProjectArchived: { project: { ...project, status: "archived" }, sessions: [] } },
    { ProjectDeleted: { project, sessions: [] } },
    { ProjectRestored: { project, sessions: [] } },
  ]
  const client = {
    send: async (request: unknown) => {
      requests.push(request)
      return responses.shift()
    },
  } as unknown as LocalIpcClient

  await listProjects(client, true)
  await renameProject(client, project.id, "Renamed")
  await updateProjectWorkspaces(client, project.id, ["/repo", "/shared"])
  await archiveProject(client, project.id)
  await deleteProject(client, project.id)
  await restoreProject(client, project.id)

  assert.deepEqual(requests, [
    { ListProjects: { include_archived: true } },
    { RenameProject: { project_id: project.id, name: "Renamed" } },
    { UpdateProjectWorkspaces: { project_id: project.id, workspace_ids: ["/repo", "/shared"] } },
    { ArchiveProject: { project_id: project.id } },
    { DeleteProject: { project_id: project.id } },
    { RestoreProject: { project_id: project.id } },
  ])
})

test("project API preserves the kernel idle blocker message", async () => {
  const client = {
    send: async () => {
      throw new Error("Project project-1 cannot be archived until all sessions are idle; active sessions: session-1")
    },
  } as unknown as LocalIpcClient

  await assert.rejects(
    archiveProject(client, "project-1"),
    /cannot be archived until all sessions are idle; active sessions: session-1/,
  )
})

function runtimeProject() {
  return {
    id: "project-1",
    owner_user_id: "owner",
    workspace_id: "/repo",
    name: "Frontend",
    kind: "named" as const,
    status: "active" as const,
    created_at_ms: 1,
    updated_at_ms: 2,
  }
}
