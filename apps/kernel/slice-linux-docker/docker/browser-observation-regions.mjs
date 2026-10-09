// MP-08/MP-10/MP-11: exact fill targets share the browser/desktop collector.
import { BrowserCdpClient } from './browser-controller-cdp.mjs';
import { measureBrowserProtection, measurePageProtection, pruneBrowserFillTargets } from './browser-protection-regions.mjs';
import { fileURLToPath } from 'node:url';
export async function locateBrowserRegions(targets,browser,values=[],{contentTarget=null,contentScale=1,onPlainField}={}) {
  const policy={targets,values,unknown:false};
  let pages;
  if(contentTarget) {
    const {connection,sessionId}=await browser.resolvePageTarget(contentTarget);
    const supplied=targets.filter(t=>t.target_id===contentTarget),tracked=[...(browser.fillTargets?.values()??[])].filter(t=>t.target_id===contentTarget);
    const merged=supplied.map(t=>tracked.find(own=>own.node_ref===t.node_ref&&own.document_id===t.document_id)??t);
    for(const target of tracked)if(!merged.includes(target))merged.push(target);
    pages=[await measurePageProtection(connection,sessionId,contentTarget,{...policy,targets:merged},{includeHidden:true,onPlainField})];
    pruneBrowserFillTargets(browser,connection);
  } else pages=(await measureBrowserProtection(browser,policy)).pages;
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
if(process.argv[1]===fileURLToPath(import.meta.url)) {
  const browser=new BrowserCdpClient({requestTimeoutMs:1500});
  try {let input='';for await(const chunk of process.stdin)input+=chunk;const policy=JSON.parse(input);
    process.stdout.write(JSON.stringify(await locateBrowserRegions(policy.targets,browser,policy.values)));
  } catch {process.stderr.write('observation redacted, retrying\n');process.exitCode=75;}
  finally {await browser.close();}
}
