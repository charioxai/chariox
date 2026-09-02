export function automationNoticeTexts(snapshot) {
  return automationNoticeEntries(snapshot).map((entry) => entry.text)
}

export function automationNoticeIds(snapshot) {
  return automationNoticeEntries(snapshot).map((entry) => entry.id)
}

export function automationNoticeEntries(snapshot) {
  const notices = new Map()
  if (snapshot?.agentPanes && typeof snapshot.agentPanes === "object") {
    for (const [agentId, entries] of Object.entries(snapshot.agentPanes)) {
      if (!Array.isArray(entries)) continue
      collectNotices(notices, agentId, entries)
    }
  }
  if (notices.size === 0 && Array.isArray(snapshot?.transcript?.entries)) {
    const visibleAgentId = typeof snapshot.transcript.visibleAgentId === "string"
      ? snapshot.transcript.visibleAgentId
      : "visible"
    collectNotices(notices, visibleAgentId, snapshot.transcript.entries)
  }
  return [...notices.values()]
}

function collectNotices(notices, agentId, entries) {
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
