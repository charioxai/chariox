#!/usr/bin/env node
// MD-DISPLAY-02/03: sequential bitrate campaign; child owns and cleans its runtime.
import {spawn} from 'node:child_process';
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {git,resources,guard} from './runtime.mjs';
const root=path.resolve(process.env.MD_CAMPAIGN_OUTPUT||`/root/.codex/evidence/browser-resume-20260930/display/phase2-${Date.now()}`);
if(root.startsWith(git('rev-parse','--show-toplevel')+'/'))throw Error('MD-DISPLAY external evidence required');
await mkdir(root,{recursive:true});
const manifest={items:['MD-DISPLAY-02','MD-DISPLAY-03'],source:git('rev-parse','HEAD'),started:new Date().toISOString(),runs:[]};
const methods=(process.env.MD_METHODS||'selkies,h264,vp9,av1').split(',');
const rates=(process.env.MD_RATES||'500000,1000000,2000000,4000000,8000000').split(',').map(Number);
let current;for(const sig of ['SIGINT','SIGTERM'])process.on(sig,()=>{current?.kill(sig);process.exitCode=130});
const cases=[];
if(methods.includes('selkies'))cases.push({name:'selkies-default',env:{MD_BASELINE:'1'}});
for(const method of methods)for(const bitrate of rates){
 const env={MD_BITRATE:String(bitrate),MD_RATE:'constant',MD_MODES:'screencast',MD_REFINE:'1'};
 if(method==='selkies')Object.assign(env,{MD_BASELINE:'1',MD_BASELINE_RATE:'cbr',MD_REFINE:'0'});
 else env.MD_CODEC={h264:'avc1.640033',vp9:'vp09.00.10.08',av1:'av01.0.08M.08'}[method];
 cases.push({name:`${method}-${bitrate}`,env});
}
for(const c of cases){
 if(process.exitCode===130)break;guard(await resources());
 const env={MD_PUBLIC:'0',MD_PAGES:process.env.MD_PAGES||'docs,spa,form,media,iframe',...c.env,MD_OUTPUT:path.join(root,c.name)};
 const record={name:c.name,env,command:['node','experiments/multidomain-display/run.mjs'],started:new Date().toISOString()};manifest.runs.push(record);
 current=spawn('node',record.command.slice(1),{env:{...process.env,...env},stdio:['ignore','pipe','pipe']});let log='';
 for(const s of [current.stdout,current.stderr])s.on('data',b=>{log+=b;process.stdout.write(b)});
 record.exit_code=await new Promise((r,j)=>{current.on('exit',(code,signal)=>{record.signal=signal;r(code)});current.on('error',j)});
 record.finished=new Date().toISOString();await writeFile(path.join(root,`${c.name}.log`),log);
 await writeFile(path.join(root,'campaign.json'),JSON.stringify(manifest,null,2));
}
manifest.finished=new Date().toISOString();await writeFile(path.join(root,'campaign.json'),JSON.stringify(manifest,null,2));
console.log(`MD-DISPLAY-02/03 campaign ${root}`);
