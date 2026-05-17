import type {
  RuntimeSession,
  WorkflowDefinition,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowCreatePayload = {
  workflow: WorkflowDefinition
  session: RuntimeSession
}

export type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

export type WorkflowLifecycleCommandContext = {
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowLifecycleCommandDeps = {
  sessionState: () => RuntimeSession
  workflowScreenActive: () => boolean
  showWorkflowScreen: () => void
  selectWorkflowCanvas: (workflowId: string | null) => void
  replaceWorkflowDefinitions: (workflows: WorkflowDefinition[]) => void
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  createWorkflow: (alias?: string | null) => Promise<WorkflowCreatePayload>
  listWorkflows: () => Promise<WorkflowDefinition[]>
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  assignWorkflowAlias: (workflowId: string, alias: string) => Promise<WorkflowDefinition | null>
  applySessionState: (session: RuntimeSession) => void
  flashFooter: (message: string, tone: FooterTone) => void
}

export async function handleWorkflowRootCommand(
  deps: WorkflowLifecycleCommandDeps,
): Promise<void> {
  const knownWorkflows = deps.sessionState().workflows ?? []
  if (knownWorkflows.length > 0) {
    if (!deps.workflowScreenActive()) {
      deps.selectWorkflowCanvas(knownWorkflows[0]?.id ?? null)
      deps.showWorkflowScreen()
    }
    return
  }

  if (!deps.workflowScreenActive()) {
    deps.showWorkflowScreen()
    return
  }

  const workflows = await deps.listWorkflows()
  if (workflows.length > 0) {
    deps.replaceWorkflowDefinitions(workflows)
    deps.selectWorkflowCanvas(workflows[0]?.id ?? null)
    return
  }

  const payload = await deps.createWorkflow(null)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.applySessionState(payload.session)
  deps.flashFooter(`created workflow ${payload.workflow.id}`, "info")
}

export async function handleWorkflowListCommand(
  deps: WorkflowLifecycleCommandDeps,
): Promise<void> {
  const workflows = await deps.listWorkflows()
  deps.replaceWorkflowDefinitions(workflows)
  deps.flashFooter(
    workflows.length === 0
      ? "no workflows in workspace"
      : `workflows: ${workflows.map(formatWorkflowListItem).join(", ")}`,
    "info",
  )
}

export async function handleWorkflowShowCommand(
  deps: WorkflowLifecycleCommandDeps,
  context: WorkflowLifecycleCommandContext,
  args: readonly string[],
): Promise<void> {
  const workflowRef = context.workflowRefOrSelected(args[1])
  if (!workflowRef) {
    deps.flashFooter("usage: /workflow show [workflow-ref]", "error")
    return
  }
  const payload = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.showWorkflowScreen()
  deps.flashFooter(`workflow ${formatWorkflowWithAlias(payload.workflow)}`, "info")
}

export async function handleWorkflowNewCommand(
  deps: WorkflowLifecycleCommandDeps,
  args: readonly string[],
): Promise<void> {
  const payload = await deps.createWorkflow(args[1] ?? null)
  deps.selectWorkflowCanvas(payload.workflow.id)
  deps.showWorkflowScreen()
  deps.applySessionState(payload.session)
  deps.flashFooter(`created workflow ${formatWorkflowWithAlias(payload.workflow)}`, "info")
}

export async function handleWorkflowAliasCommand(
  deps: WorkflowLifecycleCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRef = args[0]
  const alias = args[1]
  if (!workflowRef || !alias) {
    deps.flashFooter(workflowUsage, "error")
    return
  }

  const workflow = await deps.assignWorkflowAlias(workflowRef, alias)
  if (!workflow) {
    deps.flashFooter(`unknown workflow: ${workflowRef}`, "error")
    return
  }
  deps.upsertWorkflowDefinition(workflow)
  deps.showWorkflowScreen()
  deps.flashFooter(`workflow ${workflow.id} aliased as ${workflow.alias}`, "info")
}

export const workflowUsage = "usage: /workflow | /workflow list | /workflow show [workflow-ref] | /workflow new [alias] | /workflow run|start [workflow-ref] <endpoint-ref> [prompt] | /workflow max-turns <count|off> | /workflow run-output-schema [workflow-ref] [schema-ref|none] | /workflow intermediate-output-schema [workflow-ref] [schema-ref|none] | /workflow runs [workflow-ref] | /workflow cancel <run-ref> | /workflow resume <run-ref> | /workflow terminal [workflow-ref] | /workflow <workflow-ref> <alias> | /workflow <workflow-ref> <from-node-or-agent-ref> <to-node-or-agent-ref> | /workflow node ... | /workflow edge ... | /workflow endpoint ..."

function formatWorkflowListItem(workflow: WorkflowDefinition): string {
  return workflow.alias ? `${workflow.id} (${workflow.alias})` : workflow.id
}

function formatWorkflowWithAlias(workflow: WorkflowDefinition): string {
  return `${workflow.id}${workflow.alias ? ` (${workflow.alias})` : ""}`
}
