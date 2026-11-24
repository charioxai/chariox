export function renderInteractionCustomChoiceValue(
  value: string,
  placeholder: string,
  inputKind?: "text" | "secret" | null,
): string {
  if (!value) {
    return placeholder ? `<${placeholder}>` : ""
  }
  if (inputKind !== "secret") {
    return value
  }
  return "*".repeat(Math.max(1, Math.min(value.length, 24)))
}

/** The free-text option's line: the agent's own label (none, none shown), then the value or the agent's hint, with no text of ours. */
export function customChoiceLine(
  index: number,
  custom: { label: string; placeholder?: string | null; input_kind?: "text" | "secret" | null },
  value: string,
  selected: boolean,
  editing: boolean,
): string {
  const rendered = renderInteractionCustomChoiceValue(value, custom.placeholder ?? "", custom.input_kind)
  return `${selected ? ">" : " "} ${index + 1}.${custom.label ? `${custom.label}: ` : " "}${rendered}${editing ? "_" : ""}`
}
