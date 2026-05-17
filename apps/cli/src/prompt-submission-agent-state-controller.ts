export function createPromptSubmissionAgentStateController() {
  let submittingAgentId: string | null = null

  return {
    getSubmittingAgentId: () => submittingAgentId,
    setSubmittingAgentId: (agentId: string | null) => {
      submittingAgentId = agentId
    },
    clearSubmittingAgentId: () => {
      submittingAgentId = null
    },
  }
}
