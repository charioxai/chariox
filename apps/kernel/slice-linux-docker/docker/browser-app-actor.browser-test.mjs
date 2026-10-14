// MP-08/MP-10/MP-11: supplementary real Chromium async-ancestry regression.
// The Playwright harness is separate from production sandboxed App admission.
import assert from 'node:assert/strict';
import test from 'node:test';
const {chromium} = await import(process.env.PLAYWRIGHT_MODULE);
import {AppTabs} from './browser-controller-apps.mjs';
for (const formMode of [false, true]) test(`MP-08/MP-10/MP-11: native async App origin survives owner input (form=${formMode})`, async () => {
const browser=await chromium.launch({executablePath:'/opt/google/chrome/chrome',headless:true});
try {
 const page=await browser.newPage();page.on('pageerror',e=>console.log('PAGEERROR',e.message));const cdp=await page.context().newCDPSession(page);
 await cdp.send('Runtime.enable');
 let bridgeSource;const connection={send:async(method,params)=>{
 if(method==='Page.addScriptToEvaluateOnNewDocument')bridgeSource=params.source;
 if(method.startsWith('Browser.')||method.startsWith('Target.')||method==='Fetch.enable'||method==='Page.navigate')return method==='Target.createTarget'?{targetId:'t1'}:{};
 return cdp.send(method,params);
 }, subscribe(fn){cdp.on('Debugger.scriptParsed',params=>fn({method:'Debugger.scriptParsed',sessionId:'s1',params}));cdp.on('Runtime.bindingCalled',params=>fn({method:'Runtime.bindingCalled',sessionId:'s1',params}));cdp.on('Runtime.consoleAPICalled',params=>fn({method:'Runtime.consoleAPICalled',sessionId:'s1',params}));return()=>{};}};
 const tabs=new AppTabs({ensureConnection:async()=>connection,ensureTargetSession:async()=> 's1',closePageTarget:async()=>{}});
 await tabs.open({origin_label:'test',installation_id:'installed',assets:[{path:'index.html',content_type:'text/html',body_base64:Buffer.from('test').toString('base64')}]});
 await page.route('https://app.test.invalid/**',route=>route.fulfill({body:'<html></html>',contentType:'text/html'}));
 await page.goto('https://app.test.invalid/');
 await page.setContent((formMode ? '<form id="form">' : '') + '<button id="agent">Agent delayed</button>' + (formMode ? '</form>' : '') + '<button id="human">Human</button>');
 await cdp.send('Runtime.addBinding',{name:'__charioxAppCall'});
 const installed = await cdp.send('Runtime.evaluate',{expression:bridgeSource});assert(!installed.exceptionDetails);
 await page.evaluate(formMode=>{
 (formMode ? form : agent).addEventListener(formMode ? 'submit' : 'click',async event=>{event.preventDefault();await new Promise(resolve=>setTimeout(resolve,5600));await chariox.call('delete_todo',{origin:'agent'})});
 human.setAttribute('onclick',"chariox.call('create_todo',{origin:'human'})");
 if(formMode)setTimeout(()=>chariox.call('list_todos',{origin:'owner-background'}),5300);
 },formMode);
 const done=await tabs.beginInput('t1','agent-7');await page.click('#agent');await done();
 await page.waitForTimeout(5200);
 const end=await tabs.beginInput('t1',null);await page.click('#human');await end();
 await page.waitForTimeout(700);
  const calls=(await tabs.takeCalls()).calls;

 assert.equal(calls.length,formMode ? 3 : 2);if(formMode)assert.equal(calls.find(c=>c.params.origin==='owner-background')?.actor,undefined);assert.equal(calls.find(c=>c.params.origin==='human')?.actor,undefined);
 assert.deepEqual(calls.find(c=>c.params.origin==='agent')?.actor,{kind:'agent',id:'agent-7'});
}finally{await browser.close()}
});

// The production flow: the controller serves the App over real Fetch interception.
const text = (path, contentType, body) => ({ path, content_type: contentType, body_base64: Buffer.from(body).toString('base64') });
const appAssets = [
  text('index.html', 'text/html', '<script src="app.js" defer></script><a id="link" href="/?landed=link">link</a><button id="reload">reload</button><button id="delayed">delayed</button><button id="form">form</button>'),
  text('app.js', 'text/javascript', `
    if (location.search) chariox.call('delete_todo', { via: location.search });
    if (sessionStorage.getItem('reload')) { sessionStorage.removeItem('reload'); chariox.call('delete_todo', { via: '?landed=reload' }); }
    document.getElementById('reload').onclick = () => { sessionStorage.setItem('reload', '1'); location.reload(); };
    document.getElementById('delayed').addEventListener('click', () => setTimeout(() => { location.href = '/?landed=delayed'; }, 400));`),
];
async function mountApp() {
  const browser = await chromium.launch({ executablePath: '/opt/google/chrome/chrome', headless: true });
  const page = await browser.newPage(), cdp = await page.context().newCDPSession(page);
  const connection = { send: async (method, params) => {
    if (method.startsWith('Browser.') || method.startsWith('Target.')) return method === 'Target.createTarget' ? { targetId: 't1' } : {};
    return cdp.send(method, params);
  }, subscribe(fn) {
    for (const method of ['Debugger.scriptParsed', 'Runtime.bindingCalled', 'Runtime.consoleAPICalled', 'Page.frameNavigated', 'Fetch.requestPaused']) cdp.on(method, params => fn({ method, sessionId: 's1', params }));
    return () => {};
  } };
  await cdp.send('Page.enable'); await cdp.send('Runtime.enable');
  const tabs = new AppTabs({ ensureConnection: async () => connection, ensureTargetSession: async () => 's1', closePageTarget: async () => {} });
  await tabs.open({ origin_label: 'test', installation_id: 'installed', assets: appAssets });
  await page.waitForSelector('#link');
  return { browser, page, tabs };
}
const landed = async (tabs, via) => {
  for (let i = 0; i < 50; i++) {
    const call = tabs.calls.find(c => c.params.via === via);
    if (call) return call;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`no call landed via ${via}`);
};
const agent7 = { kind: 'agent', id: 'agent-7' };
for (const [name, selector, via, physical] of [['same-origin link', '#link', '?landed=link', true], ['location.reload()', '#reload', '?landed=reload', false], ['delayed location.href', '#delayed', '?landed=delayed', false]]) {
  test(`MP-08/MP-10/MP-11: agent ${name} keeps the agent on the destination document`, async () => {
    const { browser, page, tabs } = await mountApp();
    try {
      const done = await tabs.beginInput('t1', 'agent-7');
      await page.click(selector); await done();
      assert.deepEqual((await landed(tabs, via)).actor, agent7);
    } finally { await browser.close(); }
  });
}
test('MP-08/MP-10/MP-11: owner navigation stays owner after an agent drove the previous document', async () => {
  const { browser, page, tabs } = await mountApp();
  try {
    await (await tabs.beginInput('t1', 'agent-7'))();
    await page.click('#link');                    // direct native owner click: no beginInput
    assert.equal((await landed(tabs, '?landed=link')).actor, undefined);
  } finally { await browser.close(); }
});
test('MP-08/MP-10/MP-11: direct native owner input is not the previous agent', async () => {
  const { browser, page, tabs } = await mountApp();
  try {
    await page.evaluate(() => { document.getElementById('form').addEventListener('click', () => chariox.call('delete_todo', { via: 'direct' })); });
    await (await tabs.beginInput('t1', 'agent-7'))();
    await page.click('#form');                    // person in the headed window: no beginInput
    assert.equal((await landed(tabs, 'direct')).actor, undefined);
  } finally { await browser.close(); }
});
