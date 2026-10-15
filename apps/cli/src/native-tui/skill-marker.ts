// MP-08 / MP-10 / MP-11: timeout guard only; the kernel owns resolution.
export function hasExplicitSkillMarker(prompt: string): boolean {
  let fence: [string, number] | null = null
  let span = 0
  for (const line of String(prompt).split(/(?<=\n)/)) {
    const trimmed = line.replace(/^ */, ""), indent = line.length - trimmed.length
    const marker = trimmed[0], count = (trimmed.match(/^(`+|~+)/)?.[0] ?? "").length
    if (!span && indent <= 3 && count >= 3 && (marker === "`" || marker === "~")) {
      if (fence) { if (marker === fence[0] && count >= fence[1] && !trimmed.slice(count).trim()) fence = null }
      else fence = [marker!, count]
      continue
    }
    if (fence || (!span && indent >= 4)) continue
    let i = 0
    while (i < line.length) {
      if (span) {
        if (line[i] === "`") { const start = i; while (line[i] === "`") i++; if (i-start === span) span = 0 }
        else i++
        continue
      }
      if (line[i] === "[") {
        const close = line.indexOf("](", i), end = close < 0 ? -1 : line.indexOf(")", close+2)
        if (end >= 0) { i = end+1; continue }
      }
      if (line[i] === "\\") { i += 2; continue }
      if (line[i] === "`") { const start = i; while (line[i] === "`") i++; span = i-start; continue }
      if ((line[i] === "'" || line[i] === '"') && !(i > 0 && /[a-zA-Z0-9]/.test(line[i-1]!))) {
        const quote = line[i++]
        while (i < line.length) { if (line[i] === "\\") i += 2; else if (line[i++] === quote) break }
        continue
      }
      if (line[i] !== "$") { i++; continue }
      const start = i++
      if (start > 0 && !/[\s([]/.test(line[start-1]!)) continue
      const nameStart = i
      while (i < line.length && /[a-zA-Z0-9_-]/.test(line[i]!)) i++
      const name = line.slice(nameStart, i)
      if (!/^[a-zA-Z][a-zA-Z0-9_-]{0,127}$/.test(name) || /^[A-Z0-9_]+$/.test(name)) continue
      if (i < line.length && !/[\s)\],.!?;:]/.test(line[i]!)) continue
      if (line[i] === "]" && line[i+1] === "(") continue
      return true
    }
  }
  return false
}
