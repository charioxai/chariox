export function renderInteractionCustomChoiceValue(
  value: string,
  placeholder: string,
  inputKind?: "text" | "secret" | null,
): string {
  if (!value) {
    return `<${placeholder}>`
  }
  if (inputKind !== "secret") {
    return value
  }
  return "*".repeat(Math.max(1, Math.min(value.length, 24)))
}
