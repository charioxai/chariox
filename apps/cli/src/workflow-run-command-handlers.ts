import type {
  RuntimeSession,
  WorkflowRun,
} from "./cli-types.js"

type FooterTone = "info" | "error"

export type WorkflowRunCancelPayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

export type WorkflowRunResumePayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

export type WorkflowRunPausePayload = {
  workflow_run: WorkflowRun
  session: RuntimeSession
}

export type WorkflowRunGetPayload = {
  workflow_run: WorkflowRun
}

export type WorkflowRunCommandDeps = {
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice?: (message: string) => void
  applySessionState: (session: RuntimeSession) => void
  listWorkflowRuns?: (workflowRef?: string | null) => Promise<WorkflowRun[]>
  cancelWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunCancelPayload>
  getWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunGetPayload>
  pauseWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunPausePayload>
  resumeWorkflowRun?: (workflowRunRef: string) => Promise<WorkflowRunResumePayload>
}

export async function handleWorkflowRunShowCommand(
  deps: WorkflowRunCommandDeps,
  args: readonly string[],
): Promise<void> {
  const action = args[0] === "run-get" ? "run-get" : "run-show"
  const workflowRunRef = args[1]
  if (!workflowRunRef) {
    deps.flashFooter(`usage: /workflow ${action} <run-ref>`, "error")
    return
  }
  if (!deps.getWorkflowRun) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.getWorkflowRun(workflowRunRef)
  deps.appendNotice?.(JSON.stringify(payload.workflow_run, null, 2))
  deps.flashFooter(`workflow run ${formatWorkflowRunSummary(payload.workflow_run)}`, "info")
}

export async function handleWorkflowRunPauseCommand(
  deps: WorkflowRunCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRunRef = args[1]
  if (!workflowRunRef) {
    deps.flashFooter("usage: /workflow pause <run-ref>", "error")
    return
  }
  if (!deps.pauseWorkflowRun) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.pauseWorkflowRun(workflowRunRef)
  deps.applySessionState(payload.session)
  deps.flashFooter(
    `paused workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
    "info",
  )
}

export async function handleWorkflowRunsCommand(
  deps: WorkflowRunCommandDeps,
  args: readonly string[],
): Promise<void> {
  if (!deps.listWorkflowRuns) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const workflowRef = args[1] ?? null
  const workflowRuns = await deps.listWorkflowRuns(workflowRef)
  deps.flashFooter(
    workflowRuns.length === 0
      ? (workflowRef ? `no workflow runs for ${workflowRef}` : "no workflow runs in session")
      : `workflow runs: ${workflowRuns.map(formatWorkflowRunSummary).join(", ")}`,
    "info",
  )
}

export async function handleWorkflowRunCancelCommand(
  deps: WorkflowRunCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRunRef = args[1]
  if (!workflowRunRef) {
    deps.flashFooter("usage: /workflow cancel <run-ref>", "error")
    return
  }
  if (!deps.cancelWorkflowRun) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.cancelWorkflowRun(workflowRunRef)
  deps.applySessionState(payload.session)
  deps.flashFooter(
    `cancelled workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
    "info",
  )
}

export async function handleWorkflowRunResumeCommand(
  deps: WorkflowRunCommandDeps,
  args: readonly string[],
): Promise<void> {
  const workflowRunRef = args[1]
  if (!workflowRunRef) {
    deps.flashFooter("usage: /workflow resume <run-ref>", "error")
    return
  }
  if (!deps.resumeWorkflowRun) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.resumeWorkflowRun(workflowRunRef)
  deps.applySessionState(payload.session)
  deps.flashFooter(
    `resumed workflow run ${payload.workflow_run.id} [${String(payload.workflow_run.status).toLowerCase()}]`,
    "info",
  )
}

export function formatWorkflowRunSummary(workflowRun: WorkflowRun): string {
  const failureSummary = (workflowRun.failure_events?.length ?? 0) > 0
    ? `, failures ${workflowRun.failure_events?.length ?? 0}`
    : ""
  return `${workflowRun.id} [${String(workflowRun.status).toLowerCase()}${failureSummary}]`
}
