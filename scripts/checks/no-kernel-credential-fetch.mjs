// Core principle (row 580): no kernel depends on another kernel after installation. The one wire between kernels is the peer protocol
// (`RelayPeerRequest`, `RelayPeerResponse`, `RelayPeerEvent`). A message on it that carries a credential or asks for one is a
// credential or config fetch from another kernel at run time, unless it is an install-time transfer the owner started.
import { posix } from 'node:path'
import { closing } from '../lib/rust-scan.mjs'
import { productionSource } from '../lib/rust-production-source.mjs'

const PEER_FILES = /^apps\/kernel\/src\/transport\/relay_peer[\w]*\.rs$/
const PEER_ENUMS = /\bpub enum (RelayPeer(?:Request|Response|Event)|Peer\w+)\s*\{/g
// A variant is a candidate when its name says it moves a credential, or a field type is a credential, secret, Vault or token type.
const NAMED = /(Credential|Secret|Vault)/
const TYPE_NAME = /\b[A-Z]\w*\b/g
const MARKER = /Credential|Secret|Vault|Token/

// Install-time transfers: the owner starts them once and the receiving kernel keeps its own copy (docs/PROVIDER_ACCOUNTS.md,
// docs/CONTROL_PLANE_MACHINE_BOUNDARY.md). They never run again for a launch, a prompt or a tool call.
export const TRANSFERS = new Map([
  ['InstallProviderAccountCopy', 'owner-started provider account copy: the credential is copied once and the worker launches from its own copy'],
  ['InstallManagedSliceRelayToken', 'installation of a managed slice: the slice receives its relay token once'],
])

// Named exceptions (row 581, Miguel 11-25): a leased agent is an agent of the home kernel that operates in a worker kernel. It sees
// and uses what a local agent of the home uses, including the home Vault and Project Environment at run time (fill by handle through
// the home). If the home is down the leased agent cannot work anyway. Provider logins still transfer at install (rows 577, 582).
const LEASED_AGENT_USES_HOME = 'row 581: a leased agent is the home kernel\'s agent and uses the home Vault and Environment at run time'
export const NAMED_EXCEPTIONS = new Map([
  ['ResolveHomeCredentialSecret', LEASED_AGENT_USES_HOME],
  ['HomeCredentialSecretResolved', LEASED_AGENT_USES_HOME],
  ['InvokeHomeCredentialTool', LEASED_AGENT_USES_HOME],
  ['HomeCredentialToolHandled', LEASED_AGENT_USES_HOME],
  ['CaptureForHomeVault', LEASED_AGENT_USES_HOME],
  ['HomeVaultCapturePrepared', LEASED_AGENT_USES_HOME],
  ['PullProjectEnvironmentSecret', LEASED_AGENT_USES_HOME],
  ['ProjectEnvironmentSecretPulled', LEASED_AGENT_USES_HOME],
  ['ManagedSliceRelayTokenRefreshed', 'row 581: a managed slice keeps its relay token fresh through its home while it runs'],
])

// [{enumName, variant, line, fields}] for the variants of one enum, comments and literals hidden.
function variants(code, open, end) {
  const body = code.slice(open + 1, end)
  const out = []
  let depth = 0
  let start = 0
  for (let i = 0; i <= body.length; i++) {
    const c = body[i]
    if (c === '(' || c === '{' || c === '[') depth++
    else if (c === ')' || c === '}' || c === ']') depth--
    else if (i === body.length || (c === ',' && depth === 0)) {
      const chunk = body.slice(start, i).replace(/#\[(?:[^\[\]]|\[[^\]]*\])*\]/g, a => ' '.repeat(a.length))
      const name = /^\s*([A-Z]\w*)/.exec(chunk)
      if (name) out.push({ variant: name[1], at: open + 1 + start + chunk.indexOf(name[1]), fields: chunk.slice(name[0].length) })
      start = i + 1
    }
  }
  return out
}

export default {
  name: 'no-kernel-credential-fetch',
  why: 'present before this check existed',
  rule: 'AGENTS.md runtime rule 8: no kernel depends on another kernel after installation, so credentials and config are transferred once and never fetched or shipped per launch',
  rootFix: 'transfer the credential once as an owner-started copy (docs/PROVIDER_ACCOUNTS.md) and let the receiving kernel use its own copy; then delete the variant, or name an install-time transfer in TRANSFERS of scripts/checks/no-kernel-credential-fetch.mjs (NAMED_EXCEPTIONS holds only what the owner decided, row 581)',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files.filter(f => PEER_FILES.test(f))) {
      const text = ctx.read(file)
      if (text === null) continue
      const code = productionSource(text, file)
      for (const m of code.matchAll(PEER_ENUMS)) {
        const open = m.index + m[0].length - 1
        const end = closing(code, open)
        if (end < 0) continue
        for (const { variant, at, fields } of variants(code, open, end)) {
          if (TRANSFERS.has(variant) || NAMED_EXCEPTIONS.has(variant)) continue
          const hits = [...new Set([...(NAMED.test(variant) ? [variant] : []), ...(fields.match(TYPE_NAME) ?? []).filter(type => MARKER.test(type))])]
          if (!hits.length) continue
          findings.push({ file, line: code.slice(0, at).split('\n').length, what: `${posix.basename(file)} ${m[1]}::${variant} moves a credential between kernels (${hits.join(', ')})` })
        }
      }
    }
    return findings
  },
}
