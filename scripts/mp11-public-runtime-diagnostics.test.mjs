import test from 'node:test'
import assert from 'node:assert/strict'
import {publicProviderRun,publicRuntimeDiagnostic} from '../apps/kernel/slice-linux-docker/docker/public-runtime-diagnostics.mjs'
test('MP-11 F7 diagnostic projections omit every private execution field',()=>{
 const secret='mp11-synthetic-mcp-bearer';const run={id:'provider-run-1',state:'Ended',runtime_mcp_server_url:'http://fixture',runtime_mcp_auth_token:secret,pty_env:{UNRELATED:secret},pty_args:[secret],mcp_servers:[{token:secret}],provider_config_overrides:{value:secret}}
 for(const value of [publicProviderRun(run),publicRuntimeDiagnostic(run),publicRuntimeDiagnostic({ProviderRunLaunched:{provider_run:run}}),publicRuntimeDiagnostic({[secret]:run})]) assert.equal(JSON.stringify(value).includes(secret),false)
 assert.deepEqual(publicProviderRun(run),{id:'provider-run-1',state:'Ended',runtime_mcp_bound:true})
})

test('MP-11 F7 actual unexpected-launch diagnostic excludes private response payload', async()=>{
 const fs=await import('node:fs');const path=await import('node:path')
 const root=process.env.MP11_TEST_SOURCE??path.resolve(import.meta.dirname,'..')
 const source=fs.readFileSync(path.join(root,'apps/cli/scripts/live-external-provider-session-import-drill.mjs'),'utf8')
 const start=source.indexOf('function unwrapProviderRunLaunch(')
 const tail=source.slice(start);const end=tail.slice(1).search(/\n(?:async )?function /)+1
 const fn=new Function('publicRuntimeDiagnostic',`${tail.slice(0,end)}; return unwrapProviderRunLaunch`)(publicRuntimeDiagnostic)
 const sentinel='mp11-synthetic-mcp-bearer'
 let diagnostic='';try{fn({Unexpected:{pty_env:{INTERNAL:sentinel}}})}catch(error){diagnostic=error.message}
 assert.ok(diagnostic);assert.equal(diagnostic.includes(sentinel),false)
})
