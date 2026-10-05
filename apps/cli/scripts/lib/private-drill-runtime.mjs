import {mkdtemp,chmod,lstat,rm,open} from 'node:fs/promises'
import {constants} from 'node:fs'
import os from 'node:os'
import path from 'node:path'

export async function createPrivateDrillRuntime(label) {
  if(!/^[a-z0-9-]+$/.test(label)) throw new Error('invalid runtime label')
  const root=await mkdtemp(path.join(os.tmpdir(),`chariox-${label}-`))
  try {await chmod(root,0o700)}catch(error){await rm(root,{recursive:true,force:true});throw error}
  const identity=await lstat(root)
  return {root,async cleanup(){
    const current=await lstat(root).catch(error=>{if(error.code==='ENOENT')return null;throw error})
    if(!current)return
    if(current.isSymbolicLink()||current.dev!==identity.dev||current.ino!==identity.ino)throw new Error('private runtime ownership changed')
    await rm(root,{recursive:true})
  }}
}
export async function withPrivateDrillRuntime(label,operation) {
  const runtime=await createPrivateDrillRuntime(label)
  try{return await operation(runtime.root)}finally{await runtime.cleanup()}
}
export function requireScopedProviderPath(environment,name) {
  const value=environment[name]?.trim()
  if(!value||!path.isAbsolute(value))throw new Error(`explicit linked provider scope ${name} is required`)
  return value
}
export async function readBoundedPrivateInput(file,maximum=16*1024*1024) {
  const handle=await open(file,constants.O_RDONLY|constants.O_NOFOLLOW|constants.O_NONBLOCK)
  const buffer=Buffer.alloc(maximum+1)
  try{
    const stat=await handle.stat()
    if(!stat.isFile()||stat.size>maximum)throw new Error('provider input is not a bounded regular file')
    let size=0
    while(size<buffer.length){const {bytesRead}=await handle.read(buffer,size,buffer.length-size,null);if(!bytesRead)break;size+=bytesRead}
    if(size>maximum)throw new Error('provider input exceeds byte limit')
    return buffer.subarray(0,size).toString('utf8')
  }finally{buffer.fill(0);await handle.close()}
}

export async function copyPrivateCredential(source,destination) {
  const {randomBytes}=await import('node:crypto')
  const {mkdir,rename}=await import('node:fs/promises')
  const parent=path.dirname(destination)
  await mkdir(parent,{recursive:true,mode:0o700})
  const parentHandle=await open(parent,constants.O_RDONLY|constants.O_DIRECTORY|constants.O_NOFOLLOW)
  const pinned=process.platform==='linux'?`/proc/self/fd/${parentHandle.fd}`:parent
  const temporary=path.join(pinned,`.chariox-private-${randomBytes(16).toString('hex')}.tmp`)
  let handle,buffer
  try{
    const existing=await lstat(path.join(pinned,path.basename(destination))).catch(error=>{if(error.code==='ENOENT')return null;throw error})
    if(existing?.isSymbolicLink()||existing&&!existing.isFile())throw new Error('credential destination is aliased')
    // Bounded no-follow source admission occurs before any destination write.
    const input=await readBoundedPrivateInput(source,1024*1024)
    buffer=Buffer.from(input)
    handle=await open(temporary,constants.O_WRONLY|constants.O_CREAT|constants.O_EXCL|constants.O_NOFOLLOW,0o600)
    await handle.chmod(0o600);await handle.writeFile(buffer);await handle.sync()
    await rename(temporary,path.join(pinned,path.basename(destination)));await parentHandle.sync()
    return true
  }catch(error){if(error.code==='ENOENT')return false;throw new Error('private credential publication failed')}
  finally{buffer?.fill(0);await handle?.close();await rm(temporary,{force:true});await parentHandle.close()}
}
