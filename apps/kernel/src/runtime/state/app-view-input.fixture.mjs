// Real Chromium input and the production controller; the Rust test supplies the
// kernel, grants and backend and answers the page's pending calls over stdio.
import { pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
const [directory, root] = process.argv.slice(2);
const {KernelBrowserHost, scheduleHostRequest} = await import(pathToFileURL(join(directory,'kernel-browser-host.mjs')));
const {BrowserControllerStdioServer} = await import(pathToFileURL(join(directory,'browser-controller.mjs')));
const host = new KernelBrowserHost(root);
let sequence = 1_000_000;
let closing = false;
const stop = async () => { if (closing) return; closing = true; await host.stop(); process.exit(0); };
process.on('SIGTERM', stop);
process.on('SIGINT', stop);
async function drive(request) {
  const {params} = request;
  const tab = [...host.tabs.values()].find(tab=>tab.target_id === params.target_id);
  if (!tab) throw new Error('input target missing');
  const {connection,sessionId} = await host.browser.resolvePageTarget(tab.target_id);
  const evaluate = async expression => (await connection.send('Runtime.evaluate',{expression,returnByValue:true},sessionId)).result.value;
  for(let i=0;i<200 && !await evaluate('window.ready === true');i++) await delay(10);
  const send = async (input, agent) => {
    const reply = await host.handle({id:++sequence,method:'host.browser',params:{op:'input',
      tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id,input,
      ...(agent ? {_agent_input:true,observed_by:'agent:'+agent,_app_view_tab:tab.tab_id} : {})}});
    if (!reply.ok) throw new Error(JSON.stringify(reply.error));
  };
  const click = async (selector, agent) => {
    const point=await evaluate(`(()=>{const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return{x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)}})()`);
    await send({kind:'click',...point},agent);
  };
  if (params.name === 'human_input') await click('#human');
  else if (params.name === 'browser_click') await click('#agent',params.agent);
  else { await click('form input'); await send({kind:'key',key:'Enter'},params.agent); }
  return {id:request.id,ok:true,result:{ok:true}};
}
await new BrowserControllerStdioServer({
  handleRequest: request => request.method === 'browser.app.agent' && ['browser_click','browser_submit','human_input'].includes(request.params.name)
    ? drive(request) : host.handle(request),
  scheduleRequest: request => scheduleHostRequest(request,host.browser),
}).run().finally(()=>host.stop());
