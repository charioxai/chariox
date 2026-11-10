#!/usr/bin/env node
// MD-DISPLAY-02/03: standalone, local screenshot explorer from exact run receipts.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
const roots=process.argv.slice(2);
if(roots.length<2)throw Error('MD-DISPLAY usage: report.mjs <external-output-dir> <run-dir>...');
const output=path.resolve(roots.shift());await mkdir(output,{recursive:true});
const cases=[],sources=[];
for(const root of roots){
  const bytes=await readFile(path.join(root,'results.json')),receipt=JSON.parse(bytes);
  if(receipt.exit_code!==0||receipt.cleanup.live_owned_processes.length||!receipt.cleanup.servers_closed)throw Error('MD-DISPLAY report admits only successful, cleaned component runs');
  const prefix=path.relative(output,path.resolve(root));
  sources.push({run:path.basename(root),commit:receipt.source.commit,dirty:receipt.source.dirty,results_sha256:createHash('sha256').update(bytes).digest('hex'),start:receipt.start,finish:receipt.finish,baseline:receipt.baseline?.image_id||null,public_docs:receipt.public_docs});
  for(const row of receipt.runs){
    if(row.status!=='PASS_PROTOTYPE'||row.latency.n!==20||!row.fidelity.comparable)throw Error('MD-DISPLAY incomplete component case');
    const name=`${row.page}-${row.mode}`;
    cases.push({label:`${row.page} / ${row.mode} / ${row.codec||'DOM+PNG'}`,run:path.basename(root),page:row.page,mode:row.mode,codec:row.codec,latency:row.latency,ingress:row.latency_from_input_ingress,fidelity:row.fidelity,resources:row.resources,bytes_per_s:row.bytes_per_s,source:`${prefix}/${name}-source.png`,viewer:`${prefix}/${name}-viewer.png`,diff:`${prefix}/${name}-diff.png`});
  }
}
const data=JSON.stringify({sources,cases}).replaceAll('<','\\u003c');
const html=`<!doctype html><html><meta charset="utf-8"><title>MD-DISPLAY evidence</title><style>
body{font:16px system-ui,sans-serif;margin:24px auto;padding:0 20px;max-width:1200px;color:#182331;background:#fff}h1{font-size:26px}select,input{font:inherit}table{border-collapse:collapse;width:100%;font-size:14px}th,td{padding:8px;border-bottom:1px solid #ddd;text-align:left}pre{white-space:pre-wrap;font-size:12px}.compare{position:relative;width:min(100%,960px);aspect-ratio:8/5;border:1px solid #ddd}.compare img{position:absolute;width:100%;height:100%;object-fit:contain}#error{width:min(100%,960px)}small{color:#475569}</style>
<h1>MD-DISPLAY-02/03 — component evidence</h1><p>Headed Chromium · DPR 2 · loopback only · 20 simulated viewer inputs per cell. Video latency reads canvas pixels after rAF; DOM latency reads mirrored marker state after rAF. Both include automation and exclude physical panel scanout. Pixel metrics do not establish Vault or arbitrary-site acceptance.</p>
<label>Case <select id="case"></select></label><p id="stats"></p><label>Viewer opacity <input id="opacity" type="range" min="0" max="1" step=".01" value=".5"></label><p><small>At 0: source. At 1: viewer. Intermediate values expose geometry drift.</small></p><div class="compare"><img id="source"><img id="viewer"></div><p>4× absolute RGB error:</p><img id="error"><h2>MD-DISPLAY measurements</h2><table><thead><tr><th>Case</th><th>p50 / p95 ms</th><th>Ingress p95 ms</th><th>RGB PSNR dB</th><th>KiB/s</th><th>Host CPU %</th><th>Host RSS MiB</th></tr></thead><tbody id="rows"></tbody></table><h2>MD-DISPLAY source receipts</h2><pre id="sources"></pre>
<script>const data=${data};const select=document.querySelector('#case');const cells=(row)=>{const b=row.bytes_per_s[row.mode==='dom'?'dom':row.mode==='selkies'?'selkies_video':'encoded'];return [row.label,row.latency.p50_ms.toFixed(1)+' / '+row.latency.p95_ms.toFixed(1),row.ingress.p95_ms.toFixed(1),row.fidelity.lossless?'exact':row.fidelity.psnr_db.toFixed(2),(b/1024).toFixed(1),row.resources.cpu_percent_of_one_core.toFixed(0),(row.resources.peak_owned_host_rss_bytes/1048576).toFixed(0)]};data.cases.forEach((row,i)=>{const option=document.createElement('option');option.value=i;option.textContent=row.label+' — '+row.run;select.append(option);const tr=document.createElement('tr');cells(row).forEach(value=>{const td=document.createElement('td');td.textContent=value;tr.append(td)});document.querySelector('#rows').append(tr)});function show(){const row=data.cases[select.value];document.querySelector('#source').src=row.source;document.querySelector('#viewer').src=row.viewer;document.querySelector('#error').src=row.diff;document.querySelector('#stats').textContent=cells(row).join(' · ')}select.onchange=show;document.querySelector('#opacity').oninput=e=>document.querySelector('#viewer').style.opacity=e.target.value;document.querySelector('#viewer').style.opacity=.5;document.querySelector('#sources').textContent=JSON.stringify(data.sources,null,2);show();</script></html>`;
await writeFile(path.join(output,'index.html'),html);await writeFile(path.join(output,'summary.json'),JSON.stringify({items:['MD-DISPLAY-02','MD-DISPLAY-03'],sources,cases},null,2));
console.log(`MD-DISPLAY-02/03 ${cases.length} cleaned component cases in ${output}`);
