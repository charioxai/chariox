import {
  checkProjectEnvironmentRequest,
  environmentOperationFromResponse,
  getProjectEnvironmentRequest,
  importProjectEnvironmentEntriesRequest,
  projectEnvironmentFromResponse,
  runProjectEnvironmentSetupRequest,
} from "@chariox/kernel-client/ipc-project-environment-requests"
import { filePresenceText, secretPresenceText, setupStateText, type EnvironmentSourceKernel } from "./project-environment-projection.js"

export const projectEnvironmentActions = ["status", "update", "setup", "check"] as const
export type ProjectEnvironmentAction = (typeof projectEnvironmentActions)[number]

export type ProjectEnvironmentCommandDeps = {
  readonly send: (request: unknown) => Promise<unknown>
  /** The Project of the focused Room and the folder the Room works in. */
  readonly room: () => { readonly projectId: string; readonly checkoutPath: string }
  readonly otherKernels: () => Promise<readonly EnvironmentSourceKernel[]>
  readonly newOperationId: () => string
  readonly now: () => number
}

/** `/environment status|update [from <machine>]|setup|check`: names and states of the Room's Project Environment, as notice text. */
export async function projectEnvironmentCommand(deps: ProjectEnvironmentCommandDeps, args: readonly string[]): Promise<string> {
  const { projectId, checkoutPath } = deps.room()
  const [action, ...rest] = args
  switch (action) {
    case "status": {
      const environment = projectEnvironmentFromResponse(await deps.send(getProjectEnvironmentRequest(projectId, checkoutPath)))
      if (!environment.exists) return "This Project has no Environment yet."
      const section = (title: string, rows: readonly string[]) => rows.length === 0 ? [] : [title, ...rows.map((row) => `  ${row}`)]
      return [
        `Environment of ${environment.machineLabel} · revision ${environment.revision} · ${setupStateText(environment.setupState, deps.now())}`,
        ...section("Secrets", environment.secrets.map((secret) => `${secret.name}  ${secretPresenceText(secret)}`)),
        ...section("Files outside Git", environment.files.map((file) => `${file.path}  ${filePresenceText(file)}`)),
        ...section("MCPs", environment.mcps.map((mcp) => `${mcp.name}  ${mcp.source.kind === "file" ? mcp.source.path : "Chariox registration"}  ${mcp.presence === "present" ? "here" : "Missing here"}`)),
        ...section("Skills", environment.skills.map((skill) => `${skill.name}  ${skill.source.kind === "file" ? skill.source.path : "Chariox registration"}  ${skill.presence === "present" ? "here" : "Missing here"}`)),
        ...(environment.proposals.length > 0 ? [`${environment.proposals.length} proposal${environment.proposals.length === 1 ? "" : "s"} waiting`] : []),
      ].join("\n")
    }
    case "setup": {
      const operation = environmentOperationFromResponse(await deps.send(runProjectEnvironmentSetupRequest(projectId, deps.newOperationId(), checkoutPath)))
      return `Setup ${operation.state} (${operation.operationId})`
    }
    case "check": {
      const operation = environmentOperationFromResponse(await deps.send(checkProjectEnvironmentRequest(projectId, deps.newOperationId(), false)))
      return `Check ${operation.state} (${operation.operationId})`
    }
    case "update": {
      const from = rest[0] === "from" ? rest.slice(1).join(" ") : rest.join(" ")
      const kernels = await deps.otherKernels()
      const source = from === "" ? (kernels.length === 1 ? kernels[0] : undefined) : kernels.find((kernel) => kernel.id === from || kernel.label.toLowerCase() === from.toLowerCase())
      if (!source) return kernels.length === 0 ? "No other machine of yours is online." : `Name the machine to update from: ${kernels.map((kernel) => kernel.label).join(", ")}`
      const operation = environmentOperationFromResponse(await deps.send(importProjectEnvironmentEntriesRequest({ projectId, operationId: deps.newOperationId(), sourceKernelId: source.id, checkoutPath })))
      return `Updating from ${source.label} (${operation.operationId})`
    }
    default:
      return `usage: /environment ${projectEnvironmentActions.join(" | ")}`
  }
}
