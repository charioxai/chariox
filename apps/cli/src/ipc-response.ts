export function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  // Unit variants (e.g. `CloudRelayLoggedOut`) arrive as bare strings.
  if ((response as unknown) === variant) {
    return undefined as T
  }
  if (typeof response !== "object" || response === null || !(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
