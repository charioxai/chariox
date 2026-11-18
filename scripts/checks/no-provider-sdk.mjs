// Rule 2: providers run only through their official tools (CLIs, local servers, documented hooks), never through a provider SDK.
import { posix } from 'node:path'

const OFFICIAL_TOOLS = new Set(['@anthropic-ai/claude-code', '@openai/codex', '@google/gemini-cli'])
const SDK = /^(@anthropic-ai\/(sdk|claude-agent-sdk|claude-code-sdk|bedrock-sdk|vertex-sdk)|openai|@openai\/(agents|sdk)[\w/-]*|@google\/(generative-ai|genai)|@google-cloud\/vertexai|@ai-sdk\/[\w-]+|ai|langchain|@langchain\/[\w-]+|cohere-ai|@mistralai\/[\w-]+)$/
const RUST_SDK = /^(async-openai|anthropic|anthropic-sdk|misanthropy|genai|rig-core|openai-api-rs|google-generative-ai-rs)$/
const PY_SDK = /^(anthropic|openai|google-generativeai|google-genai|cohere|mistralai|langchain[\w-]*|claude-agent-sdk)$/i
const SOURCE = /\.(m?[jt]sx?|cjs|cts|mts)$/

function lineOf(text, index) {
  return text.slice(0, index).split('\n').length
}

export default {
  name: 'no-provider-sdk',
  why: 'present before this check existed',
  rule: 'AGENTS.md runtime rule 2: providers run only through their official tools, no provider SDKs or third-party agent services',
  rootFix: 'drive the provider through its official CLI, local server or documented hook (docs/PROVIDER_ACCOUNTS.md) and delete the SDK dependency',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      // The planted-violation tests of the checks hold SDK imports inside strings.
      if (file.startsWith('scripts/checks/')) continue
      const name = posix.basename(file)
      const text = ctx.read(file)
      if (text === null) continue
      if (name === 'package.json') {
        const manifest = (() => { try { return JSON.parse(text) } catch { return {} } })()
        for (const table of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies']) {
          for (const dep of Object.keys(manifest[table] ?? {})) {
            if (SDK.test(dep) && !OFFICIAL_TOOLS.has(dep)) findings.push({ file, line: lineOf(text, text.indexOf(`"${dep}"`)), what: `${table} lists the provider SDK ${dep}` })
          }
        }
      } else if (name === 'Cargo.toml') {
        for (const m of text.matchAll(/^\s*([\w-]+)\s*=/gm)) {
          if (RUST_SDK.test(m[1])) findings.push({ file, line: lineOf(text, m.index), what: `Cargo manifest lists the provider SDK crate ${m[1]}` })
        }
      } else if (/^requirements.*\.txt$/.test(name) || name === 'pyproject.toml') {
        for (const m of text.matchAll(/^\s*"?([A-Za-z][\w.-]*)\s*(?:[=<>~!;[\s"',]|$)/gm)) {
          if (PY_SDK.test(m[1])) findings.push({ file, line: lineOf(text, m.index), what: `Python requirements list the provider SDK ${m[1]}` })
        }
      } else if (SOURCE.test(name)) {
        for (const m of text.matchAll(/(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\(\s*)['"]([^'"]+)['"]/g)) {
          const pkg = m[1].replace(/^((?:@[^/]+\/)?[^/]+).*$/, '$1')
          if (SDK.test(pkg) && !OFFICIAL_TOOLS.has(pkg)) findings.push({ file, line: lineOf(text, m.index), what: `imports the provider SDK ${pkg}` })
        }
      }
    }
    return findings
  },
}
