export type ProtocolMinimumDiagnostic = {
  capability: string
  requestVariant: string
  nestedVariant?: string
  minimumProtocolVersion: number
}

export async function withProtocolMinimum<T>(
  operation: () => Promise<T>,
  diagnostic: ProtocolMinimumDiagnostic,
): Promise<T> {
  try {
    return await operation()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    const variants = diagnostic.nestedVariant
      ? [diagnostic.requestVariant, diagnostic.nestedVariant]
      : [diagnostic.requestVariant]
    if (!variants.some((variant) => isUnknownRequestVariant(message, variant))) throw error
    throw new Error(
      `${diagnostic.capability} requires kernel protocol ${diagnostic.minimumProtocolVersion} or newer: ${message}`,
    )
  }
}

export function sendWithProtocolMinimum<TResponse>(
  send: <T>(request: unknown) => Promise<T>,
  request: unknown,
  diagnostic: ProtocolMinimumDiagnostic,
): Promise<TResponse> {
  return withProtocolMinimum(
    () => send<TResponse>(request),
    diagnostic,
  )
}

function isUnknownRequestVariant(message: string, requestVariant: string): boolean {
  const escapedVariant = requestVariant.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
  return new RegExp(
    "unknown variant\\s+[`'\"]?" + escapedVariant + "(?:[`'\"]|\\b)",
    "i",
  ).test(message)
}
