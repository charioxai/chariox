// MP-08 / MP-10 / MP-11: allowlisted product status; credentials stay in the client.
import { pathToFileURL } from 'node:url'
import path from 'node:path'
const [runtimeRoot, endpoint, label, provider] = process.argv.slice(2)
const { LocalIpcClient } = await import(pathToFileURL(path.join(runtimeRoot, 'packages/kernel-client/dist/ipc.js')))
const client = new LocalIpcClient(endpoint)
try {
  const response = await client.send({ ListProviderAccountProfiles: { provider } })
  const profile = response.ProviderAccountProfilesListed?.profiles?.find(p => p.label === label)
  if (!Array.isArray(response.ProviderAccountProfilesListed?.profiles)) throw new Error('profiles_unavailable')
  if (process.argv.includes('--diagnostic')) {
    process.stdout.write(JSON.stringify({ profiles: response.ProviderAccountProfilesListed.profiles.map(p => ({ provider: p.provider, profile_id: p.profile_id, label: p.label, auth_state: p.auth_state })) }) + '\n')
  } else if (process.argv.includes('--ready')) {
    process.stdout.write(JSON.stringify({ ready: true }) + '\n')
  } else {
  if (!profile) throw new Error('profile_missing')
  if (process.argv.includes('--refresh')) {
    await client.send({ RefreshProviderAccountProfile: { provider, account_profile: profile.profile_id } })
    const refreshed = await client.send({ ListProviderAccountProfiles: { provider } })
    Object.assign(profile, refreshed.ProviderAccountProfilesListed?.profiles?.find(p => p.profile_id === profile.profile_id) ?? {})
  }
  process.stdout.write(JSON.stringify({
    provider: profile.provider, profile_id: profile.profile_id, auth_state: profile.auth_state,
    meters: (profile.usage?.meters ?? []).map(m => ({ kind: m.kind, state: m.state, resets_at_ms: m.resets_at_ms, observed_at_ms: m.observed_at_ms })),
  }) + '\n')
  }
} catch {
  process.stderr.write('MP-08 / MP-10: product-linked account status unavailable\n')
  process.exitCode = 1
} finally { await client.close() }
