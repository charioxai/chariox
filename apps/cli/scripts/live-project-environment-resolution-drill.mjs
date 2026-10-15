#!/usr/bin/env node
// MP-02 / MP-08 / MP-10 / MP-11: real protocol 524/peer132 boundary.
// Supplementary protocol drill; hosted normal-click/TUI acceptance is separate.
import assert from 'node:assert/strict'
import {mkdtemp, mkdir, realpath, rm, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {fileURLToPath, pathToFileURL} from 'node:url'
import {createHash} from 'node:crypto'

const args = process.argv.slice(2)
if(args.includes('--help')) {
 console.log('MP-02 / MP-08 / MP-10 / MP-11: node live-project-environment-resolution-drill.mjs --kernel ws://127.0.0.1:PORT/kernel --scratch-root /absolute/external/scratch --evidence /absolute/external/result.json --confirm-disposable-kernel\nBuild @chariox/kernel-client first, or set CHARIOX_KERNEL_CLIENT_MODULE to the exact built ipc.js. Uses the existing local authentication. Creates/deletes only its two new Projects; never starts a provider or session.')
 process.exit(0)
}
const value = name => args[args.indexOf(name)+1]
assert(args.includes('--kernel') && args.includes('--scratch-root') && args.includes('--evidence') && args.includes('--confirm-disposable-kernel'), 'explicit disposable-kernel, external scratch and evidence arguments required')
const endpoint = new URL(value('--kernel'))
assert(endpoint.protocol === 'ws:' && ['127.0.0.1','localhost','[::1]'].includes(endpoint.hostname) && endpoint.pathname === '/kernel', 'local disposable kernel endpoint required')
const repo = await realpath(path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..'))
const external = async input => {
 assert(path.isAbsolute(input), 'absolute external path required')
 const resolved=await realpath(input)
 assert(resolved !== repo && !resolved.startsWith(repo+path.sep), 'scratch/evidence cannot be inside the checkout')
 return resolved
}
const scratchRoot=await external(value('--scratch-root'))
const evidence=path.join(await external(path.dirname(value('--evidence'))),path.basename(value('--evidence')))
assert(!path.relative(scratchRoot,evidence).split(path.sep).every(part=>part!=='..'), 'evidence must survive scratch cleanup')
const scratch=await mkdtemp(path.join(scratchRoot,'project-resolution-'))
const modulePath=process.env.CHARIOX_KERNEL_CLIENT_MODULE ?? path.join(repo,'packages/kernel-client/dist/ipc.js')
assert(path.isAbsolute(modulePath),'absolute built client module required')
const {LocalIpcClient}=await import(pathToFileURL(modulePath).href)
const client=new LocalIpcClient(endpoint.href)
const owned=[]
const report={mp:['MP-02','MP-08','MP-10','MP-11'],started:new Date().toISOString(),clientModule:modulePath,clientHash:createHash('sha256').update(await import('node:fs/promises').then(fs=>fs.readFile(modulePath))).digest('hex'),assertions:[],cleanup:[],result:'FAIL'}
const resolve=async(workspace,projectId,create)=>{
 const reply=await client.send({ResolveProject:{workspace_id:workspace,project_id:projectId,create_if_missing:create}})
 assert(reply && 'ProjectResolved' in reply,'524 ProjectResolved response required')
 return reply.ProjectResolved.project
}
try {
 const a=path.join(scratch,'source'),b=path.join(scratch,'other-project')
 const before=(await client.send({ListProjects:{include_archived:true}})).ProjectsListed.projects.map(p=>p.id).sort()
 const sessionsBefore=JSON.stringify((await client.send({ListSessions:null})).SessionsListed.sessions)
 await mkdir(a);await mkdir(b)
 assert.equal(await resolve(a,null,false),null)
 assert.equal(await resolve(a,null,false),null)
 const afterLookup=(await client.send({ListProjects:{include_archived:true}})).ProjectsListed.projects
 for(const p of afterLookup)if(!before.includes(p.id) && p.workspace_id===a)owned.push(p.id)
 assert.deepEqual(afterLookup.map(p=>p.id).sort(),before)
 report.assertions.push('lookup without mutation')
 const project=await resolve(a,null,true)
 if(project?.id)owned.push(project.id)
 assert(project?.id && project.standalone && project.workspace_id===a)
 const retry=await resolve(a,null,true)
 assert.equal(retry.id,project.id)
 report.assertions.push('explicit standalone Set up and idempotent retry')
 const other=await resolve(b,null,true);owned.push(other.id)
 await assert.rejects(resolve(a,other.id,true),/caller does not own|workspace/i)
 report.assertions.push('foreign Project/workspace binding refused')
 const env=await client.send({GetProjectEnvironment:{projectId:project.id}})
 assert.equal(env.ProjectEnvironment.environment.local_project_id,project.id)
 report.assertions.push('Environment read uses kernel-returned Project identity')
 assert.equal(JSON.stringify((await client.send({ListSessions:null})).SessionsListed.sessions),sessionsBefore)
 report.assertions.push('zero sessions or provider runs created')
 report.result='PASS'
} catch(error) {
 report.error=error instanceof Error ? error.message : 'protocol drill failed'
 throw error
} finally {
 for(const id of owned.reverse()) {
  try {await client.send({DeleteProject:{project_id:id}});report.cleanup.push({project:id,removed:true})}
  catch {report.cleanup.push({project:id,removed:false});report.result='FAIL'}
 }
 await client.close()
 await rm(scratch,{recursive:true})
 report.finished=new Date().toISOString()
 await writeFile(evidence,JSON.stringify(report,null,2),{mode:0o600})
 console.log(`MP-02 / MP-08 / MP-10 / MP-11: ${report.result}; evidence ${evidence}`)
 if(report.result!=='PASS')process.exitCode=1
}
