// Names the owner retired. A tracked file may keep today's matches (counted in retired-names.allow.json); a new match anywhere is red.
const RETIRED = [
  { pattern: /waiting[ _-]?room/i, name: 'waiting room', use: 'the plain name of the thing (inbox, queue or the agent state it shows)' },
]

export default {
  name: 'retired-names',
  why: 'written before the name was retired; rename when touching the file',
  rule: 'Owner naming decision: retired names are never written (waiting room, 2026-11-16)',
  rootFix: 'use the current name; when you touch a file that still has the old one, rename it there and run node scripts/checks/run.mjs --shrink',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (file.startsWith('scripts/checks/')) continue
      for (const { pattern, name } of RETIRED) {
        if (pattern.test(file)) findings.push({ file, line: 0, what: `the file name uses the retired name "${name}"` })
      }
    }
    // One pattern pass over the whole tree is far cheaper than reading every file.
    const joined = new RegExp(RETIRED.map(r => `(${r.pattern.source})`).join('|'), 'i')
    for (const hit of ctx.grep(joined.source, { ignoreCase: true, paths: [':!scripts/checks'] })) {
      const name = RETIRED.find(r => r.pattern.test(hit.text))?.name ?? RETIRED[0].name
      findings.push({ file: hit.file, line: hit.line, what: `uses the retired name "${name}"` })
    }
    return findings
  },
}
