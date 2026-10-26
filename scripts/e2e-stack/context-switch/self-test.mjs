// MP-08 / MP-10 / MP-11: supplementary fresh-checkout packaging regression.
import {readdir,readFile,access,mkdir,mkdtemp,rm,realpath} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {homedir} from 'node:os';
import {resolve,relative,isAbsolute,join} from 'node:path';
if(process.env.NODE_TEST_CONTEXT)throw Error('Run this self-test directly with node; nested node --test can skip its child tests');
const root=new URL('./',import.meta.url);
for(const name of await readdir(root)){
 if(!name.endsWith('.mjs')||name==='self-test.mjs')continue;
 const file=new URL(name,root),source=await readFile(file,'utf8');
 const checked=spawnSync(process.execPath,['--check',fileURLToPath(file)],{encoding:'utf8'});
 if(checked.status!==0)throw Error(checked.stderr);
 for(const m of source.matchAll(/(?:from\s*|import\s*\(|new URL\s*\()(['"])(\.[^'"]+)\1/g))await access(new URL(m[2],file));
 if(/(?:\/root\/work\/agent-|agents\/ctxfix2\/r\d+\/harness|\/w\/valenv\/tools)/.test(source))throw Error('Round-specific dependency: '+name);
}
const tests=(await readdir(root)).filter(name=>name.endsWith('.test.mjs')).sort();
const parent=resolve(process.env.TMPDIR??join(homedir(),'.chariox/dev/context-switch/tmp'));
const repository=await realpath(fileURLToPath(new URL('../../../',root)));
function requireExternal(path){const rel=relative(repository,path);if(rel===''||(!rel.startsWith('../')&&!isAbsolute(rel)))throw Error('Self-test TMPDIR must be outside the repository');}
requireExternal(parent);
await mkdir(parent,{recursive:true,mode:0o700});
const canonical=await realpath(parent);requireExternal(canonical);
const temp=await mkdtemp(join(canonical,'self-test-'));
try{
 const run=spawnSync(process.execPath,['--test','--test-concurrency=2',...tests.map(x=>fileURLToPath(new URL(x,root)))],{stdio:'inherit',timeout:30000,env:{...process.env,TMPDIR:temp}});
 if(run.status!==0)process.exitCode=1;
 else console.log('MP-08 / MP-10 / MP-11: fresh-checkout supplementary self-test PASS');
}finally{await rm(temp,{recursive:true,force:true});}
