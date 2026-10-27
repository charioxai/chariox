export class FakeAgent {
  constructor({ agentId, harness, interactions, cwd }) {
    this.agentId = agentId
    this.harness = harness
    this.interactions = interactions
    this.cwd = cwd
  }

  async execute(command, permissionMode = "limited", turnId = null) {
    return await this.harness.execute({
      agentId: this.agentId,
      command,
      cwd: this.cwd,
      permissionMode,
      turnId,
    })
  }

  async requestChoice({ title, message, choices, defaultChoice = null, turnId = null }) {
    return await this.interactions.request({
      agentId: this.agentId,
      turnId,
      kind: "choice",
      severity: "info",
      title,
      message,
      choices: choices.map((choice) => typeof choice === "string" ? { id: choice, label: choice } : choice),
      defaultChoice,
    })
  }

  async requestPopup({
    title,
    message,
    level = "info",
    choices,
    defaultChoice = null,
    timeoutSec = null,
    turnId = null,
  }) {
    return await this.interactions.request({
      agentId: this.agentId,
      turnId,
      kind: "choice",
      severity: level,
      title,
      message,
      choices: choices.map((choice) => typeof choice === "string"
        ? { id: choice, label: choice, reply: choice }
        : choice),
      defaultChoice,
      timeoutSec,
    })
  }
}
