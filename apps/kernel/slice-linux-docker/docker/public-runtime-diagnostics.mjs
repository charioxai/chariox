const states = new Set(['Starting','Running','Parked','Ended','starting','running','parked','ended','failed','error'])
const variants = new Set(['ProviderRun','ProviderRunLaunched','ProviderRunLaunchAccepted','Agent','AgentSpawned','SessionCreated','Error','ProviderRunNotFound'])
const publicId = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,128}$/.test(value) ? value : null
export function publicProviderRun(run) {
  return {id:publicId(run?.id),state:states.has(run?.state)?run.state:null,
    runtime_mcp_bound:Boolean(run?.runtime_mcp_server_url&&run?.runtime_mcp_auth_token)}
}
export function publicRuntimeResponse(value) {
  const variant = [...variants].find(key => value && typeof value === 'object' && Object.hasOwn(value,key))
  const body = variant ? value[variant] : value
  return {variant:variant??'unknown',provider_run:publicProviderRun(body?.provider_run??body)}
}
export function publicRuntimeDiagnostic(value) {return JSON.stringify(publicRuntimeResponse(value))}
