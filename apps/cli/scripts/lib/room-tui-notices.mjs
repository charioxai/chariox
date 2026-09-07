export function roomActionNoticePattern(action) {
  if (!Number.isSafeInteger(action.sequence) || action.sequence < 1
    || !["browser", "computer"].includes(action.mode) || !/^[a-z_]+$/.test(action.kind)) {
    throw new Error("invalid Room action notice identity")
  }
  let outcome = "completed"
  if (action.state === "failed") {
    if (action.outcome?.status !== "failed" || !["controller_failure", "process_lost"].includes(action.outcome.code)) {
      throw new Error("invalid Room failure notice outcome")
    }
    outcome = `failed \\(${action.outcome.code}\\)`
  } else if (action.state !== undefined && action.state !== "completed") {
    throw new Error("Room notice requires a completed or failed action")
  }
  return new RegExp(`^Room action #${action.sequence}: .+ · ${action.mode} ${action.kind} · ${outcome}$`)
}

export function automationNoticeTexts(snapshot) {
  return automationNoticeEntries(snapshot).map((entry) => entry.text)
}

export function automationNoticeIds(snapshot) {
  return automationNoticeEntries(snapshot).map((entry) => entry.id)
}

export function automationNoticeEntries(snapshot) {
  const candidates = []
  let paneCandidates = 0
  if (snapshot?.agentPanes && typeof snapshot.agentPanes === "object") {
    for (const [agentId, entries] of Object.entries(snapshot.agentPanes)) {
      if (!Array.isArray(entries)) continue
      candidates.push([agentId, entries])
      paneCandidates += 1
    }
  }
  if (paneCandidates === 0 && Array.isArray(snapshot?.transcript?.entries)) {
    const visibleAgentId = typeof snapshot.transcript.visibleAgentId === "string"
      ? snapshot.transcript.visibleAgentId
      : "visible"
    candidates.push([visibleAgentId, snapshot.transcript.entries])
  }
  const notices = new Map()
  for (const [agentId, entries] of candidates) {
    for (const entry of entries) {
      if (
        entry?.role !== "notice"
        || (typeof entry.id !== "string" && typeof entry.id !== "number")
        || typeof entry.text !== "string"
      ) continue
      const id = `${agentId}:${entry.id}`
      notices.set(id, { id, text: entry.text })
    }
  }
  return [...notices.values()]
}
