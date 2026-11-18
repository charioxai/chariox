// Rule 3: the relay only carries encrypted traffic. It never reads prompts, outputs or data and never decides session state.
import { posix } from 'node:path'

const RELAY = 'apps/relay/'
// Crates that decrypt or open payloads. The relay authenticates with HMAC and forwards ciphertext, so none of these belongs in it.
const PAYLOAD_CRATES = /^(chacha20poly1305|aes-gcm|aes-gcm-siv|aead|xsalsa20poly1305|crypto_box|age|ring|openssl|snow|hpke|orion)$/
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
