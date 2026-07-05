import type { AgentInstance, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import type {
  WorkflowPromptState as SharedWorkflowPromptState,
  WorkflowPromptSubmitDecision,
} from "@arroba/kernel-client/workflow-prompt-state"

export {
  deriveWorkflowPromptState,
  formatWorkflowAgentLabel,
  formatWorkflowPromptPlaceholder,
  isWorkflowCommandInput,
  resolveActiveWorkflowRun,
  resolveSelectedWorkflow,
  resolveSelectedWorkflowNodeId,
  validateWorkflowPromptSubmit,
} from "@arroba/kernel-client/workflow-prompt-state"

export type WorkflowPromptState = SharedWorkflowPromptState<WorkflowDefinition, WorkflowRun, AgentInstance>
export type { WorkflowPromptSubmitDecision }
