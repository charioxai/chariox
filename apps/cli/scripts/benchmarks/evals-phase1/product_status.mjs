// MP-08 / MP-10 / MP-11: allowlisted product status; credentials stay in the client.
import { pathToFileURL } from 'node:url'
import path from 'node:path'
const [runtimeRoot, endpoint, label, provider] = process.argv.slice(2)
const { LocalIpcClient } = await import(pathToFileURL(path.join(runtimeRoot, 'packages/kernel-client/dist/ipc.js')))
const client = new LocalIpcClient(endpoint)
try {
  const response = await client.send({ ListProviderAccountProfiles: { provider } })
  const profile = response.ProviderAccountProfilesListed?.profiles?.find(p => p.label === label)
  if (!profile) throw new Error('profile_missing')
  process.stdout.write(JSON.stringify({
    provider: profile.provider, profile_id: profile.profile_id, auth_state: profile.auth_state,
    meters: (profile.usage?.meters ?? []).map(m => ({ kind: m.kind, state: m.state, resets_at_ms: m.resets_at_ms, observed_at_ms: m.observed_at_ms })),
  }) + '\n')
} catch {
  process.stderr.write('MP-08 / MP-10: product-linked account status unavailable\n')
  process.exitCode = 1
} finally { await client.close() }
