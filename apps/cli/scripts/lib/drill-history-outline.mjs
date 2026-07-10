export function historyOutlineRows(outline, { includeUserPrompt = false } = {}) {
  return (outline.agents ?? [])
    .flatMap((agent) => agent.turns ?? [])
    .flatMap((turn) => [
      ...(includeUserPrompt ? [turn.user_prompt] : []),
      ...(turn.entries ?? []),
      ...(turn.summary ? [turn.summary] : []),
      ...(turn.blobs ?? []).map((blob) => ({ entry: { text: blob.summary } })),
    ])
    .filter(Boolean)
}

export function historyOutlineText(outline, options = {}) {
  return historyOutlineRows(outline, options)
    .map((row) => row.entry?.text ?? "")
    .join("\n")
}

export function historyOutlineContiguousText(outline, options = {}) {
  return historyOutlineRows(outline, options)
    .map((row) => row.entry?.text ?? "")
    .join("")
}
