// Safety rule: state, logs and test data never live in a repository (they belong in ~/.chariox/dev/<task>/ or outside it).
const STATE_DIR = /(^|\/)(evidence|\.chariox|\.playwright-mcp|\.claude|\.codex)\//
const STATE_FILE = /\.(log|mp4|mov|webm|mkv|har|pcap|sqlite3?|db)$/i
const IMAGE = /\.(png|jpe?g|gif|webp)$/i
const IMAGE_LIMIT = 200 * 1024

export default {
  name: 'tracked-state',
  why: 'committed before this check existed; delete it from the repository',
  rule: 'AGENTS.md state, secrets and safety: never put state, logs or test data in a repository; keep screenshots, recordings and logs outside it',
  rootFix: 'delete the file from the repository (git rm) and keep it under ~/.chariox/dev/<task>/ or in the evidence folder outside the checkout',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (STATE_DIR.test(file)) findings.push({ file, line: 0, what: `tracked state directory content (${file.match(STATE_DIR)[2]})` })
      else if (STATE_FILE.test(file)) findings.push({ file, line: 0, what: 'tracked log, recording or database file' })
      else if (IMAGE.test(file) && ctx.size(file) > IMAGE_LIMIT) findings.push({ file, line: 0, what: `tracked image over ${IMAGE_LIMIT / 1024} KB (${Math.round(ctx.size(file) / 1024)} KB)` })
    }
    return findings
  },
}
