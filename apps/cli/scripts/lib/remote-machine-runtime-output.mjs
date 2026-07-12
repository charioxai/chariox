function normalizeKind(kind) {
  return String(kind ?? "")
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
}

function recordText(record) {
  if (Array.isArray(record?.bytes)) return Buffer.from(record.bytes).toString("utf8")
  return String(record?.text ?? record?.data ?? record?.output ?? "")
}

export function terminalProviderOutputSnapshot(events, agentId = null) {
  let providerText = ""
  const errors = []
  const statuses = []
  let recordCount = 0

  for (const event of events) {
    if (event?.event !== "terminal_output" || !Array.isArray(event.records)) continue
    for (const record of event.records) {
      if (agentId && record?.agent_id && record.agent_id !== agentId) continue
      const kind = normalizeKind(record?.kind)
      const text = recordText(record)
      recordCount += 1
      if (kind === "provider_output") providerText += text
      else if (kind === "provider_error") errors.push(text)
      else if (kind === "provider_status") statuses.push(text)
    }
  }

  return { providerText, errors, statuses, recordCount }
}

export function fatalProviderOutput(snapshot) {
  if (snapshot.errors.some((text) => text.trim().length > 0)) {
    return snapshot.errors.filter(Boolean).join("\n")
  }
  const fatalStatus = snapshot.statuses.find((text) => (
    /system\s*error|\bfailed\b|unauthorized|not logged in|rate[ -]?limit/i.test(text)
  ))
  return fatalStatus ?? null
}
