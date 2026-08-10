const BULLET = " • "
const ELLIPSIS = "..."

const COMPACT_REPLACEMENTS: Array<[RegExp, string]> = [
  [/\bCLIs connected\b/g, "CLIs"],
  [/\bCLI connected\b/g, "CLI"],
  [/\bvisible agents\b/g, "agents"],
  [/\bvisible agent\b/g, "agent"],
  [/\bcollaborator agents\b/g, "collab agents"],
  [/\bcollaborators\b/g, "collabs"],
  [/\bCtrl\+C to stop\b/g, "Ctrl+C stop"],
  [/\bTab cycles focus\b/g, "Tab focus"],
  [/\bCtrl\+P opens workflow\b/g, "Ctrl+P workflow"],
  [/\bhotkeys\b/g, "keys"],
  [/\barrows move\b/g, "arrows"],
  [/\bEnter confirms\b/g, "Enter"],
  [/\bA archives\b/g, "A archive"],
  [/\bD deletes inactive\b/g, "D delete"],
  [/\bsync managed /g, "sync "],
]

const LOW_PRIORITY_SEGMENTS = [
  /^Ctrl\+P workflow$/i,
  /^Tab focus$/i,
  /^R restore$/i,
  /^A archive$/i,
  /^D delete$/i,
  /^arrows$/i,
]

export function compactFooterSummary(summary: string, maxWidth: number | null | undefined): string {
  const width = Math.floor(maxWidth ?? 0)
  if (width <= 0 || summary.length <= width) {
    return summary
  }

  let compact = applyCompactReplacements(summary)
  if (compact.length <= width) {
    return compact
  }

  let segments = compact.split(BULLET).filter((segment) => segment.length > 0)
  for (const removable of LOW_PRIORITY_SEGMENTS) {
    if (segments.length <= 2) {
      break
    }
    const nextSegments = segments.filter((segment) => !removable.test(segment))
    if (nextSegments.length !== segments.length) {
      segments = nextSegments
      compact = segments.join(BULLET)
      if (compact.length <= width) {
        return compact
      }
    }
  }

  if (segments.length > 2) {
    const first = segments[0] ?? ""
    const last = segments.at(-1) ?? ""
    compact = `${first}${BULLET}${last}`
    if (compact.length <= width) {
      return compact
    }
  }

  return truncateWithEllipsis(compact, width)
}

function applyCompactReplacements(value: string): string {
  let compact = value
  for (const [pattern, replacement] of COMPACT_REPLACEMENTS) {
    compact = compact.replace(pattern, replacement)
  }
  return compact
}

function truncateWithEllipsis(value: string, width: number): string {
  if (width <= ELLIPSIS.length) {
    return ELLIPSIS.slice(0, Math.max(0, width))
  }
  return `${value.slice(0, width - ELLIPSIS.length)}${ELLIPSIS}`
}
