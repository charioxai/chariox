// Rule 3: the relay only carries encrypted traffic. It never reads prompts, outputs or data and never decides session state.
import { posix } from 'node:path'
import { productionSource } from '../lib/rust-production-source.mjs'
import { closing } from '../lib/rust-scan.mjs'

const RELAY = 'apps/relay/'
// Crates that decrypt or open payloads. The relay authenticates with HMAC and forwards ciphertext, so none of these belongs in it.
const PAYLOAD_CRATES = /^(chacha20poly1305|aes-gcm|aes-gcm-siv|aead|xsalsa20poly1305|crypto_box|age|ring|openssl|snow|hpke|orion)$/
// The relay's payload field is `ciphertext` (apps/relay/src/protocol.rs). It is moved, cloned, base64-coded and measured with `.len()`,
// and never parsed, decoded to text, searched, formatted or logged. A statement that does either is a payload inspection.
const PAYLOAD_FIELD = /\bciphertext\b(?!\s*\.len\(\))/
const INSPECTION = /serde_json::from_|\bfrom_utf8\w*|to_string_lossy|\bformat!|\b(?:e?print(?:ln)?|write(?:ln)?|panic|debug|info|warn|error|trace)!|(?:tracing|log|logging)::|\.(?:contains|starts_with|ends_with|find|trim|parse|as_str)\s*(?:::<[^>]*>)?\(/
const isTest = file => /(^|\/)(tests?|benches)\//.test(file) || /(^|\/)[\w-]*tests?\.rs$/.test(file)

export default {
  name: 'relay-isolation',
  why: 'present before this check existed',
  rule: 'AGENTS.md runtime rule 3: the relay only carries encrypted traffic and never reads or decides anything',
  rootFix: 'keep payload parsing, decryption and session logic in the kernel or the shared protocol; the relay forwards ciphertext by envelope only',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files.filter(f => f.startsWith(RELAY))) {
      const text = ctx.read(file)
      if (text === null) continue
      if (posix.basename(file) === 'Cargo.toml') {
        const dependencies = text.split(/^\[(.*)\]\s*$/m)
        for (let i = 1; i < dependencies.length; i += 2) {
          if (!/^(build-)?dependencies$/.test(dependencies[i])) continue
          for (const m of dependencies[i + 1].matchAll(/^\s*([\w-]+)\s*=\s*(.*)$/gm)) {
            const line = text.slice(0, text.indexOf(m[0])).split('\n').length
            if (/path\s*=/.test(m[2])) findings.push({ file, line, what: `the relay depends on the workspace crate ${m[1]} by path` })
            else if (m[1].startsWith('chariox-')) findings.push({ file, line, what: `the relay depends on ${m[1]}` })
            else if (PAYLOAD_CRATES.test(m[1])) findings.push({ file, line, what: `the relay depends on the decryption crate ${m[1]}` })
          }
        }
      } else if (file.endsWith('.rs') && !isTest(file)) {
        const code = productionSource(text, file)
        let start = 0
        for (let i = 0; i < code.length; i++) {
          // A brace-delimited macro (`tracing::debug! { .. }`) is one statement with its arguments: skip to its closing brace.
          if (code[i] === '{' && /!\s*$/.test(code.slice(start, i))) { const end = closing(code, i); if (end > 0) i = end; continue }
          if (code[i] !== ';' && code[i] !== '{' && code[i] !== '}') continue
          const statement = code.slice(start, i)
          if (PAYLOAD_FIELD.test(statement) && INSPECTION.test(statement)) {
            const lead = statement.match(/^\s*/)[0]
            findings.push({ file, line: code.slice(0, start + lead.length).split('\n').length, what: `relay code inspects the payload field ciphertext: ${statement.trim().replace(/\s+/g, ' ').slice(0, 80)}` })
          }
          start = i + 1
        }
        text.split('\n').forEach((row, i) => {
          if (/^\s*\/\//.test(row)) return
          const use = /\b(?:use|extern crate)\s+(chariox_(?!relay)\w+)/.exec(row) ?? /\b(chariox_(?!relay)\w+)::/.exec(row)
          if (use) findings.push({ file, line: i + 1, what: `relay code uses ${use[1]}` })
        })
      }
    }
    return findings
  },
}
