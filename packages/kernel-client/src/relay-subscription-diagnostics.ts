export type RelaySubscriptionDiagnostic = {
  event: "binding_sent" | "event_decrypted" | "event_decrypt_failed"
  subscriptionId: string
}

/** Public binding identity and outcome only; never payloads, keys or error text. */
export class RelaySubscriptionDiagnostics {
  private readonly handlers = new Set<(diagnostic: RelaySubscriptionDiagnostic) => void>()

  subscribe(handler: (diagnostic: RelaySubscriptionDiagnostic) => void): () => void {
    this.handlers.add(handler)
    return () => { this.handlers.delete(handler) }
  }

  emit(event: RelaySubscriptionDiagnostic["event"], subscriptionId: string): void {
    for (const handler of this.handlers) {
      try { handler({ event, subscriptionId }) } catch { /* Observers cannot interrupt transport. */ }
    }
  }
}
