// Rule 6: hosted connections use wss:// behind Caddy. A plain ws:// URL in deploy or container config is allowed only on loopback.
const CONFIG = /(^|\/)(deploy|docker)\/.*\.(ya?ml|json|toml|env|service|conf)$|(^|\/)[^/]*compose[^/]*\.ya?ml$|(^|\/)Caddyfile[^/]*$|(^|\/)\.env[^/]*$/
const LOOPBACK = /^ws:\/\/(127\.\d+\.\d+\.\d+|localhost|\[::1\]|0\.0\.0\.0)(?![\w.-])/

export default {
  name: 'hosted-wss',
  why: 'docker lab compose: the lab relay is a container on a private compose network, not a hosted endpoint',
  rule: 'AGENTS.md runtime rule 6: hosted connections use wss:// behind Caddy; hosted settings that break this must fail loudly',
  rootFix: 'put the endpoint behind Caddy and configure the wss:// URL (ws:// is for loopback only)',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files.filter(f => CONFIG.test(f))) {
      const text = ctx.read(file)
      if (text === null) continue
      text.split('\n').forEach((row, i) => {
        for (const m of row.matchAll(/\bws:\/\/[^\s'"`)>,]*/g)) {
          if (!LOOPBACK.test(m[0])) findings.push({ file, line: i + 1, what: `plain ${m[0]} in deploy config` })
        }
      })
    }
    return findings
  },
}
