// MP-08/MP-10/MP-11: exact fill targets share the browser/desktop collector.
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { measureBrowserProtection, measurePageProtection, pruneBrowserFillTargets } from './browser-protection-regions.mjs';
import {execFile} from 'node:child_process';
import {mkdtemp,copyFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fenceBrowserCapture} from './browser-protection-regions.mjs';
import { fileURLToPath } from 'node:url';
export async function locateBrowserRegions(targets,browser,values=[],{contentTarget=null,contentScale=1,onPlainField,modelCapture=false}={}) {
  const policy={targets,values,unknown:false};
  let pages;
  if(contentTarget) {
    const {connection,sessionId}=await browser.resolvePageTarget(contentTarget);
    const supplied=targets.filter(t=>t.target_id===contentTarget),tracked=[...(browser.fillTargets?.values()??[])].filter(t=>t.target_id===contentTarget);
    const merged=supplied.map(t=>tracked.find(own=>own.node_ref===t.node_ref&&own.document_id===t.document_id)??t);
    for(const target of tracked)if(!merged.includes(target))merged.push(target);
    pages=[await measurePageProtection(connection,sessionId,contentTarget,{...policy,targets:merged},{includeHidden:true,onPlainField,modelCapture})];
    pruneBrowserFillTargets(browser,connection);
  } else pages=(await measureBrowserProtection(browser,policy,{modelCapture})).pages;
  const regions=[];
  for(const page of pages) {
    if(!page)continue;
    if(contentTarget) {if(page.dpr!==contentScale)throw Error('MP-11: capture density changed');regions.push(...page.regions);continue;}
    if(!page.regions.length)continue;
    if(page.dpr!==1)throw Error('MP-11: desktop placement requires AT-SPI');
    const bounds=page.window,origin=[bounds[0],bounds[1]+bounds[3]-page.viewport[1]];
    regions.push(...page.regions.map(([x,y,w,h])=>[x+origin[0],y+origin[1],w,h]));
  }
  return regions;
}
// MP-08/MP-11: Room screenshot and OCR use the same document/compositor fence
// as host captures. The Python adapter places CDP regions with live AT-SPI
// geometry, including DPR2; only a verified result may leave private scratch.
export async function captureRoomObservation(browser,policy,mode,argument,{execute=executeCapture,scratch=tmpdir()}={}) {
  if(!['screenshot','ocr','find-text'].includes(mode))throw Error('MP-11: invalid observation');
  const root=await mkdtemp(path.join(scratch,'chariox-room-observation-'));
  try {
    const destination=path.join(root,'masked.png');
    const result=await fenceBrowserCapture(browser,policy,async browser_protection=>{
      // execFile's async interface has no stdin option; provide the private
      // policy through its pipe, never command arguments or environment.
      return execute(process.env.CHARIOX_SLICE_PYTHON??'/usr/bin/python3',
        [fileURLToPath(new URL('./slice-observation-mask.py',import.meta.url)),'--measured',mode,mode==='screenshot'?destination:argument??''],
        {input:JSON.stringify({policy,browser_protection}),timeout:45000,maxBuffer:4*1024*1024});
    });
    if(mode==='screenshot')await copyFile(destination,argument);
    return result;
  } finally {await rm(root,{recursive:true,force:true});}
}

// A bounded private pipe, with diagnostics discarded by the public entrypoint.
function executeCapture(file,args,{input,...options}) {
  return new Promise((resolve,reject)=>{
    const child=execFile(file,args,options,(error,stdout,stderr)=>error?reject(error):resolve({stdout,stderr}));
    child.stdin.on('error',()=>{});child.stdin.end(input);
  });
}
if(process.argv[1]===fileURLToPath(import.meta.url)) {
  const browser=new BrowserCdpClient({requestTimeoutMs:5000});
  try {
    let input='';for await(const chunk of process.stdin)input+=chunk;const policy=JSON.parse(input);
    if(process.argv[2]==='--capture') {
      const result=await captureRoomObservation(browser,policy,process.argv[3],process.argv[4],{execute:executeCapture});
      process.stdout.write(result.stdout);
      // Native lifecycle receipts contain XIDs only. Never forward arbitrary
      // helper diagnostics, page text or policy values.
      for(const receipt of result.stderr.matchAll(/^CHARIOX_OBSERVATION_PRUNED_NATIVE:\[[0-9,]*\]$/gm))process.stderr.write(receipt[0]+'\n');
    } else process.stdout.write(JSON.stringify(await locateBrowserRegions(policy.targets,browser,policy.values)));
  } catch {process.stderr.write('observation redacted, retrying\n');process.exitCode=75;}
  finally {await browser.close();}
}
