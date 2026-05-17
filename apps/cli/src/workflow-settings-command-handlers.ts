import type {
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
  WorkflowDefinition,
} from "./cli-types.js"

const WORKFLOW_MAX_TURNS_CONFIG_KEY = "workflow.max_turns"
const WORKFLOW_LAUNCH_POLICY_CONFIG_KEY = "workflow.launch_policy"

const WORKFLOW_SETTINGS_COMMANDS = [
  "launch-policy",
  "flush-context",
  "run-output-schema",
  "intermediate-output-schema",
  "max-turns",
] as const

type WorkflowSettingsCommand = typeof WORKFLOW_SETTINGS_COMMANDS[number]
type FooterTone = "info" | "error"

type WorkflowResolvePayload = {
  workflow: WorkflowDefinition
}

export type WorkflowSettingsCommandContext = {
  firstWorkflowArgIsExplicit: (workflowRef: string | undefined) => boolean
  selectedWorkflowRef: () => string | null
  workflowRefOrSelected: (workflowRef: string | null | undefined) => string | null
}

export type WorkflowSettingsCommandDeps = {
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  flashFooter: (message: string, tone: FooterTone) => void
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  resolveWorkflow: (workflowRef: string) => Promise<WorkflowResolvePayload>
  upsertWorkflowDefinition: (workflow: WorkflowDefinition) => void
  setWorkflowFlushContext?: (
    workflowRef: string,
    flushAgentContextBeforeRun: boolean,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowLaunchPolicy?: (policy: "reject" | "queue") => Promise<{ session: RuntimeSession }>
  setWorkflowRunOutputSchema?: (
    workflowRef: string,
    runOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
  setWorkflowIntermediateOutputSchema?: (
    workflowRef: string,
    intermediateOutputSchemaRef: string | null,
  ) => Promise<{ workflow: WorkflowDefinition; session: RuntimeSession }>
}

export function isWorkflowSettingsCommand(subcommand: string | undefined): subcommand is WorkflowSettingsCommand {
  return WORKFLOW_SETTINGS_COMMANDS.includes(subcommand as WorkflowSettingsCommand)
}

export async function handleWorkflowSettingsCommand(
  deps: WorkflowSettingsCommandDeps,
  context: WorkflowSettingsCommandContext,
  args: readonly string[],
): Promise<void> {
  const subcommand = args[0]
  if (subcommand === "launch-policy") {
    await handleWorkflowLaunchPolicyCommand(deps, args)
    return
  }
  if (subcommand === "flush-context") {
    await handleWorkflowFlushContextCommand(deps, context, args)
    return
  }
  if (subcommand === "run-output-schema") {
    await handleWorkflowSchemaCommand(deps, context, args, {
      commandName: "run-output-schema",
      currentValue: (workflow) => workflow.run_output_schema_ref,
      setValue: deps.setWorkflowRunOutputSchema,
      usage: "usage: /workflow run-output-schema [workflow-ref] [schema-ref|none]",
    })
    return
  }
  if (subcommand === "intermediate-output-schema") {
    await handleWorkflowSchemaCommand(deps, context, args, {
      commandName: "intermediate-output-schema",
      currentValue: (workflow) => workflow.intermediate_output_schema_ref,
      setValue: deps.setWorkflowIntermediateOutputSchema,
      usage: "usage: /workflow intermediate-output-schema [workflow-ref] [schema-ref|none]",
    })
    return
  }
  if (subcommand === "max-turns") {
    await handleWorkflowMaxTurnsCommand(deps, args)
  }
}

async function handleWorkflowLaunchPolicyCommand(
  deps: WorkflowSettingsCommandDeps,
  args: readonly string[],
): Promise<void> {
  const value = args[1]?.trim().toLowerCase()
  if (!value) {
    deps.flashFooter(`workflow launch policy: ${currentWorkflowLaunchPolicy(deps.sessionState())}`, "info")
    return
  }
  if (value !== "reject" && value !== "queue") {
    deps.flashFooter("usage: /workflow launch-policy <reject|queue>", "error")
    return
  }
  if (!deps.setWorkflowLaunchPolicy) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.setWorkflowLaunchPolicy(value)
  deps.applySessionState(payload.session)
  deps.flashFooter(`workflow launch policy set to ${value}`, "info")
}

async function handleWorkflowFlushContextCommand(
  deps: WorkflowSettingsCommandDeps,
  context: WorkflowSettingsCommandContext,
  args: readonly string[],
): Promise<void> {
  const firstArg = args[1]?.trim().toLowerCase()
  const selectedRef = context.selectedWorkflowRef()
  const firstArgIsValue = firstArg === "true" || firstArg === "false"
  const workflowRef = context.workflowRefOrSelected(firstArgIsValue ? null : args[1])
  const value = (firstArgIsValue ? args[1] : args[2])?.trim().toLowerCase()
  if (!workflowRef) {
    deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
    return
  }
  if (firstArgIsValue && !selectedRef) {
    deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
    return
  }
  const resolved = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)
  if (!value) {
    deps.flashFooter(
      `workflow ${resolved.workflow.id} flush-context: ${(resolved.workflow.flush_agent_context_before_run ?? true) ? "true" : "false"}`,
      "info",
    )
    return
  }
  if (value !== "true" && value !== "false") {
    deps.flashFooter("usage: /workflow flush-context [workflow-ref] [true|false]", "error")
    return
  }
  if (!deps.setWorkflowFlushContext) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const payload = await deps.setWorkflowFlushContext(resolved.workflow.id, value === "true")
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(
    `workflow ${payload.workflow.id} flush-context set to ${payload.workflow.flush_agent_context_before_run ? "true" : "false"}`,
    "info",
  )
}

async function handleWorkflowSchemaCommand(
  deps: WorkflowSettingsCommandDeps,
  context: WorkflowSettingsCommandContext,
  args: readonly string[],
  config: {
    commandName: "run-output-schema" | "intermediate-output-schema"
    currentValue: (workflow: WorkflowDefinition) => string | null | undefined
    setValue: ((workflowRef: string, schemaRef: string | null) => Promise<{
      workflow: WorkflowDefinition
      session: RuntimeSession
    }>) | undefined
    usage: string
  },
): Promise<void> {
  const explicitWorkflowRef = context.firstWorkflowArgIsExplicit(args[1]) ? args[1] : null
  const workflowRef = context.workflowRefOrSelected(explicitWorkflowRef)
  const value = explicitWorkflowRef ? args[2] : args[1]
  if (!workflowRef) {
    deps.flashFooter(config.usage, "error")
    return
  }
  const resolved = await deps.resolveWorkflow(workflowRef)
  deps.upsertWorkflowDefinition(resolved.workflow)
  if (value === undefined) {
    deps.flashFooter(
      `workflow ${resolved.workflow.id} ${config.commandName}: ${config.currentValue(resolved.workflow) ?? "none"}`,
      "info",
    )
    return
  }
  if (!config.setValue) {
    deps.flashFooter("workflow runtime commands unavailable", "error")
    return
  }
  const schemaRef = value.trim().toLowerCase() === "none" ? null : value
  const payload = await config.setValue(resolved.workflow.id, schemaRef)
  deps.applySessionState(payload.session)
  deps.upsertWorkflowDefinition(payload.workflow)
  deps.flashFooter(
    `workflow ${payload.workflow.id} ${config.commandName} set to ${config.currentValue(payload.workflow) ?? "none"}`,
    "info",
  )
}

async function handleWorkflowMaxTurnsCommand(
  deps: WorkflowSettingsCommandDeps,
  args: readonly string[],
): Promise<void> {
  const value = args[1]
  if (!value) {
    const current = deps.sessionState().config_state?.values?.[WORKFLOW_MAX_TURNS_CONFIG_KEY]
    const label = current && current.trim() !== "" ? current : "unset"
    deps.flashFooter(`workflow max turns: ${label}`, "info")
    return
  }
  const attachment = deps.attachmentState()
  if (!attachment) {
    deps.flashFooter("must be attached to set workflow max turns", "error")
    return
  }
  const normalized = value.trim().toLowerCase()
  const nextValue =
    normalized === "off" || normalized === "0"
      ? "0"
      : Number.isFinite(Number(normalized))
        ? String(Math.max(1, Math.floor(Number(normalized))))
        : null
  if (!nextValue) {
    deps.flashFooter("usage: /workflow max-turns <count|off>", "error")
    return
  }
  const payload = await deps.updateSessionConfig(
    deps.sessionState().id,
    attachment.id,
    { [WORKFLOW_MAX_TURNS_CONFIG_KEY]: nextValue },
    false,
  )
  deps.applySessionState(payload.session)
  deps.flashFooter(
    nextValue === "0"
      ? "workflow max turns disabled"
      : `workflow max turns set to ${nextValue}`,
    "info",
  )
}

function currentWorkflowLaunchPolicy(session: RuntimeSession): "reject" | "queue" {
  const policy =
    session.workflow_launch_policy ??
    session.config_state?.values?.[WORKFLOW_LAUNCH_POLICY_CONFIG_KEY] ??
    "reject"
  return policy === "queue" ? "queue" : "reject"
}
