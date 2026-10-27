// MP-10/MP-11: prove foreign Docker proxy ownership using public network metadata.
export function confirmedForeignProxyPids(listeners, containers, runId, proxyArgs) {
 const confirmed=new Set()
 for(const listener of listeners) {
  const listenerPort=Number(listener.trim().split(/\s+/)[3]?.match(/:(\d+)$/)?.[1])
  for(const match of listener.matchAll(/pid=(\d+),/g)) {
   const pid=Number(match[1]),args=proxyArgs.get(pid)
   if(!args)continue
   const value=flag=>args[args.indexOf(flag)+1]
   if(!args.includes('-proto')||value('-proto')!=='tcp'||Number(value('-host-port'))!==listenerPort)continue
   if(containers.some(c=>!c.name.includes(runId)&&c.ips.includes(value('-container-ip'))&&c.bindings.some(b=>b.hostPort===listenerPort&&b.containerPort===Number(value('-container-port'))&&(!b.hostIp||b.hostIp===value('-host-ip')))))confirmed.add(pid)
  }
 }
 return confirmed
}

export async function readForeignProxyOwnership(listeners, containerRows, runId, docker, readFile) {
 const containers=[]
 for(const row of containerRows) {
  const [id,name]=row.split('|')
  if(!name.startsWith('chariox-slice-')||name.includes(runId))continue
  try {
   // Never inspect environment, mounts, credentials or provider state.
   const output=await docker(['inspect','--format','{{range .NetworkSettings.Networks}}{{.IPAddress}} {{.GlobalIPv6Address}} {{end}}|{{json .HostConfig.PortBindings}}',id])
   const [addresses,encoded]=output.split('|'),ports=JSON.parse(encoded),bindings=[]
   for(const [target,entries] of Object.entries(ports??{}))if(target.endsWith('/tcp'))for(const binding of entries??[])bindings.push({hostIp:binding.HostIp,hostPort:Number(binding.HostPort),containerPort:Number(target.split('/')[0])})
   containers.push({id,name,ips:addresses.trim().split(/\s+/).filter(Boolean),bindings})
  }catch{/* A concurrently removed foreign container cannot confirm ownership. */}
 }
 const proxyArgs=new Map(),ceasedPids=new Set()
 for(const listener of listeners)if(listener.includes('docker-proxy'))for(const match of listener.matchAll(/pid=(\d+),/g)) {
  const pid=Number(match[1])
  try {if((await readFile(`/proc/${pid}/comm`,'utf8')).trim()==='docker-proxy')proxyArgs.set(pid,(await readFile(`/proc/${pid}/cmdline`,'utf8')).split('\0').filter(Boolean))}catch(error){if(error.code==='ENOENT'||error.code==='ESRCH')ceasedPids.add(pid)}
 }
 return {confirmedPids:confirmedForeignProxyPids(listeners,containers,runId,proxyArgs),ceasedPids,containers}
}
