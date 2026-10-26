// MP-08 / MP-10 / MP-11: public paths and exact build provenance only.
import {homedir} from 'node:os';
import {resolve,relative,isAbsolute} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {readFile,realpath} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
export const harness=fileURLToPath(new URL('.',import.meta.url)).replace(/\/$/,'');
export const repository=resolve(harness,'../../..');
export const sourceTree=resolve(process.env.CTXVAL_TREE??repository);
export const lane=resolve(process.env.CTXVAL_LANE??homedir()+'/.chariox/dev/context-switch/assets');
export const stateRoot=resolve(process.env.CTXVAL_STATE_ROOT??homedir()+'/.chariox/dev/context-switch/state');
export const evidence=resolve(process.env.CTXVAL_EVIDENCE??homedir()+'/.codex/evidence/context-switch');
export const owner=process.env.CTXVAL_OWNER??'context-switch';
if(!/^[a-z0-9-]+$/.test(owner))throw Error('Invalid context-switch resource owner');
export const clientModule=name=>pathToFileURL(lane+'/client/apps/cli/dist/'+name).href;
export function playwrightModule(){if(!process.env.PLAYWRIGHT_MODULE)throw Error('PLAYWRIGHT_MODULE must name an installed Playwright module');return pathToFileURL(resolve(process.env.PLAYWRIGHT_MODULE)).href;}
function contains(parent,child){const r=relative(parent,child);return r===''||(!r.startsWith('../')&&!isAbsolute(r));}
export async function verifyBuild(label,tree){
 const canonicalTree=await realpath(tree);
 for(const p of [lane,stateRoot,evidence]){
  // Resolve existing ancestors too, so a symlink cannot put private state in Git.
  let ancestor=p,tail=[];
  for(;;){try{ancestor=await realpath(ancestor);break;}catch(e){if(e.code!=='ENOENT')throw e;const next=resolve(ancestor,'..');if(next===ancestor)throw e;tail.unshift(ancestor.split('/').at(-1));ancestor=next;}}
  const canonical=resolve(ancestor,...tail);
  if(contains(canonicalTree,canonical)||contains(repository,canonical)||contains(canonical,canonicalTree))throw Error('Build, state and evidence paths must be separate from source checkouts');
 }
 const build=JSON.parse(await readFile(lane+'/bin/build-'+label+'.json','utf8'));
 const head=execFileSync('git',['-C',tree,'rev-parse','HEAD'],{encoding:'utf8'}).trim();
 if(build.head!==head||!/^[a-f0-9]{40}$/.test(head)||!Number.isInteger(build.protocol))throw Error('Exact build/source provenance mismatch');
 if((await readFile(lane+'/bin/ready-'+label,'utf8')).trim()!==head)throw Error('Ready marker differs from source head');
 if(build.client?.head!==head)throw Error('Client must be built from the same head');
 for(const [name,digest] of Object.entries(build.sha256??{})){
  if(!['kernel-'+label,'relay-'+label,'chariox'].includes(name)||!/^[a-f0-9]{64}$/.test(digest))throw Error('Invalid public binary manifest');
  const actual=createHash('sha256').update(await readFile(lane+'/bin/'+name)).digest('hex');
  if(actual!==digest)throw Error('Built binary hash mismatch: '+name);
 }
 if(Object.keys(build.sha256??{}).length!==3)throw Error('All three built binaries must be hashed');
 return build;
}
