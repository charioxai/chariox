let nextInteractionId = 1

export class InteractionGateway {
  constructor(options = {}) {
    this.options = options
    this.history = []
    this.pending = new Map()
    this.pendingResolvers = new Map()
  }

  async request(request) {
    const interaction = {
      interactionId: request.interactionId ?? `interaction-${nextInteractionId++}`,
      turnId: request.turnId ?? null,
      agentId: request.agentId ?? null,
      kind: request.kind ?? "choice",
      severity: request.severity ?? "info",
      title: request.title ?? "Question",
      message: request.message ?? "",
      choices: request.choices ?? [],
      defaultChoice: request.defaultChoice ?? null,
      timeoutSec: Number.isFinite(request.timeoutSec) ? request.timeoutSec : null,
      continueOnTimeout: request.continueOnTimeout ?? false,
      createdAtMs: Date.now(),
    }
    this.history.push(interaction)
    this.pending.set(interaction.interactionId, interaction)
    try {
      const response = await this.#resolve(interaction)
      return {
        interactionId: interaction.interactionId,
        status: response?.status ?? "answered",
        choiceId: response?.choiceId ?? interaction.defaultChoice ?? null,
        reply: response?.reply ?? this.#replyForChoice(interaction, response?.choiceId ?? interaction.defaultChoice ?? null),
        metadata: response?.metadata ?? null,
      }
    } finally {
      this.pending.delete(interaction.interactionId)
      this.pendingResolvers.delete(interaction.interactionId)
    }
  }

  async #resolve(interaction) {
    if (typeof this.options.onRequest === "function") {
      return await this.options.onRequest(interaction)
    }
    const scripted = Array.isArray(this.options.responses) ? this.options.responses : []
    if (scripted.length > 0) {
      const next = scripted.shift()
      if (typeof next === "string") return { choiceId: next }
      return next ?? { choiceId: interaction.defaultChoice }
    }
    if (interaction.timeoutSec != null) {
      return await this.#waitForExternalResolution(interaction, interaction.timeoutSec * 1000)
    }
    return await this.#waitForExternalResolution(interaction, null)
  }

  resolveInteraction(interactionId, response = {}) {
    const resolver = this.pendingResolvers.get(interactionId)
    if (!resolver) return false
    resolver(response)
    return true
  }

  async #waitForExternalResolution(interaction, timeoutMs) {
    return await new Promise((resolve) => {
      let timeout = null
      this.pendingResolvers.set(interaction.interactionId, (response) => {
        if (timeout) clearTimeout(timeout)
        resolve(this.#normalizeResolution(interaction, response))
      })
      if (timeoutMs != null) {
        timeout = setTimeout(() => {
          this.pendingResolvers.delete(interaction.interactionId)
          if (interaction.defaultChoice != null) {
            resolve({
              status: "timed_out",
              choiceId: interaction.defaultChoice,
              reply: this.#replyForChoice(interaction, interaction.defaultChoice),
            })
            return
          }
          resolve({
            status: "timed_out",
            choiceId: null,
            reply: null,
          })
        }, timeoutMs)
      }
    })
  }

  #normalizeResolution(interaction, response) {
    if (typeof response === "string") {
      return {
        status: "answered",
        choiceId: response,
        reply: this.#replyForChoice(interaction, response),
      }
    }
    return {
      status: response?.status ?? "answered",
      choiceId: response?.choiceId ?? interaction.defaultChoice ?? null,
      reply: response?.reply ?? this.#replyForChoice(interaction, response?.choiceId ?? interaction.defaultChoice ?? null),
      metadata: response?.metadata ?? null,
    }
  }

  #replyForChoice(interaction, choiceId) {
    if (!choiceId) return null
    const match = interaction.choices.find((choice) => choice?.id === choiceId)
    return match?.reply ?? null
  }
}
