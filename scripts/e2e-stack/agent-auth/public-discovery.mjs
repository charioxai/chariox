// MP-08 / MP-09 / MP-10 / MP-11 AU-15: read-only public prerequisite probe.
// No registration, issuer minting, credential or account side effect.
const root = 'https://www.firecrawl.dev'
const get = async path => {
  const response = await fetch(root + path, { signal: AbortSignal.timeout(20000), redirect: 'error' })
  if (!response.ok) throw new Error(`Public discovery returned HTTP ${response.status}`)
  return path.endsWith('.md') ? response.text() : response.json()
}
const [resource, server, skill] = await Promise.all([
  get('/.well-known/oauth-protected-resource'), get('/.well-known/oauth-authorization-server'), get('/auth.md'),
])
const types = server.agent_auth?.identity_types_supported ?? []
const supported = types.some(type => ['anonymous', 'service_auth'].includes(type))
console.log(JSON.stringify({ MP: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], id: 'apps-autonomy/AU-15', date: new Date().toISOString(),
  status: supported ? 'NOT RUN' : 'BLOCKED(coordinator: Firecrawl requires trusted WorkOS ID-JAG; issuer work is parked)',
  resource: resource.resource, identity_types_supported: types, auth_md_fetched: skill.includes('Firecrawl'), registration_attempted: false,
}, null, 2))
if (!supported) process.exitCode = 2
