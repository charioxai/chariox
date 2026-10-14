// MP-08 / MP-10 / MP-11: supplementary fresh-checkout packaging regression.
import {readdir,readFile,access} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=new URL('./',import.meta.url);
for(const name of await readdir(root)){
 if(!name.endsWith('.mjs')||name==='self-test.mjs')continue;
 const file=new URL(name,root),source=await readFile(file,'utf8');
 const checked=spawnSync(process.execPath,['--check',fileURLToPath(file)],{encoding:'utf8'});
 if(checked.status!==0)throw Error(checked.stderr);
 for(const m of source.matchAll(/(?:from\s*|import\s*\(|new URL\s*\()(['"])(\.[^'"]+)\1/g))await access(new URL(m[2],file));
 if(/(?:\/root\/work\/agent-|agents\/ctxfix2\/r\d+\/harness|\/w\/valenv\/tools)/.test(source))throw Error('Round-specific dependency: '+name);
}
const tests=['context-switch-validation-records.test.mjs','context-switch-validation-brief.test.mjs','shaped-proxy.test.mjs','provider-proxy.test.mjs','log-observe.test.mjs','runtime-owned.test.mjs','paths.test.mjs'];
const run=spawnSync(process.execPath,['--test',...tests.map(x=>fileURLToPath(new URL(x,root)))],{stdio:'inherit',timeout:30000});
if(run.status!==0)process.exitCode=1;
else console.log('MP-08 / MP-10 / MP-11: fresh-checkout supplementary self-test PASS');
