// Commits and pull requests: a commit message is a short summary with no Co-Authored-By line and no agent or model attribution.
// Diff-aware: it reads the commits between the merge base and HEAD, so history already on main needs no list.
// Only attribution lines are matched, never a bare model or provider name: a subject about the Claude provider is fine.
// Review after 2026-12-15: widen the patterns if a new tool adds its own attribution line.
import { gitOut } from '../lib/ratchet.mjs'

const ATTRIBUTION = [
  [/^\s*co-authored-by\s*:/im, 'a Co-Authored-By line'],
  [/^\s*(?:generated|created|written|authored|assisted)\s+(?:with|by)\b.*(?:claude|codex|gemini|gpt|copilot|cursor|chatgpt|anthropic|openai)/im, 'a "generated with" attribution line'],
  [/\u{1F916}\s*generated/iu, 'a robot "generated" footer'],
  [/noreply@(?:anthropic|openai)\.com/i, 'an agent noreply address'],
]
const RECORD = '\u0001'
const FIELD = '\u0000'

// The range base..HEAD is only the branch's commits when history connects them. A shallow clone cuts that link (a pull-request
// run checks out a synthetic merge commit at depth 1), and git then lists no branch commit at all: refuse instead of passing.
function requireHistoryBack(root, base) {
  try { gitOut(root, 'merge-base', base, 'HEAD') } catch {
    const shallow = gitOut(root, 'rev-parse', '--is-shallow-repository') === 'true'
    throw new Error(shallow
      ? `this checkout is shallow, so the commits between ${base.slice(0, 12)} and HEAD cannot be read and no message was checked. Fetch the history: actions/checkout with fetch-depth: 0, or git fetch --unshallow`
      : `${base.slice(0, 12)} and HEAD have no common history, so the branch's commits cannot be read and no message was checked. Fetch the branch it was cut from or pass a base that HEAD descends from`)
  }
}

export default {
  name: 'commit-message-rules',
  ratchet: 'diff',
  rule: 'AGENTS.md "Commits and pull requests": a commit message is a short summary; no Co-Authored-By lines, no agent or model names, no logs',
  rootFix: 'reword the commit so its message is the subject (and a few short lines if needed) with no attribution line: git rebase the branch and amend, then force-push the branch with --force-with-lease',
  scan(ctx) {
    if (!ctx.base) return []
    requireHistoryBack(ctx.root, ctx.base)
    const log = gitOut(ctx.root, 'log', '--no-merges', '--format=%H%x00%s%x00%b%x01', `${ctx.base}..HEAD`)
    const findings = []
    for (const record of log.split(RECORD).map(r => r.trim()).filter(Boolean)) {
      const [sha, subject, body = ''] = record.split(FIELD)
      const message = `${subject}\n${body}`
      for (const [pattern, name] of ATTRIBUTION) {
        if (pattern.test(message)) findings.push({ file: `commit ${sha.slice(0, 10)}`, line: 0, what: `message "${subject.slice(0, 60)}" carries ${name}` })
      }
    }
    return findings
  },
}
