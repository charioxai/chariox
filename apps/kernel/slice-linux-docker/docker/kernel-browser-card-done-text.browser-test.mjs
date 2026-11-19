// AU-03: a card with done_text closes by itself when the page shows that text, through real Chrome input.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
import {startChallengeService} from '../../../../scripts/e2e-stack/vault-autonomy/challenge-service.mjs';

test('AU-03 a done_text card closes when the page shows the text, a card without it stays', {timeout:120_000}, async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE,'explicit installed Chromium required');
  const root=await mkdtemp(path.join(tmpdir(),'done-text-chrome-')),service=await startChallengeService();
  const host=new KernelBrowserHost(root,{appStorageRoot:path.join(root,'apps')}),scope='agent:done-text-chrome';
  try {
    const challenge=service.create(),opened=await host.request({op:'open',url:challenge.url,observed_by:scope});
    const binding={tab_id:opened.tab_id,generation:opened.generation,observed_by:scope};
    const snapshot=(await host.request({op:'snapshot',...binding})).snapshot;
    const main=snapshot.dom_nodes.find(n=>n.node_name==='MAIN');assert.ok(main);
    const done={op:'popup_region',...binding,card_id:'done-text-card',ref:main.node_ref,done_text:'Verification accepted'};
    const first=await host.request(done);assert.equal(first.state,'present');assert.equal(first.region.done_text,'Verification accepted');
    const click=async id=>{
      const node=(await host.request({op:'snapshot',...binding})).snapshot.dom_nodes.find(n=>n.attributes.id===id);assert.ok(node?.bounds);
      const {x,y,width,height}=node.bounds;
      await host.request({op:'input',...binding,document_id:snapshot.document_id,input:{kind:'click',x:Math.round(x+width/2),y:Math.round(y+height/2)}});
    };
    const input=value=>host.request({op:'input',...binding,document_id:snapshot.document_id,input:value});
    await click('issue');await delay(100);
    assert.equal((await host.request({...done,document_id:snapshot.document_id})).state,'present','the step is not done yet');
    // A rejected code makes the page rerender its form: the card follows it and keeps waiting for the text.
    await click('code');await input({kind:'text',text:'111111'});await input({kind:'key',key:'Enter'});
    for(let i=0;i<10;i++){await delay(100);assert.equal((await host.request({...done,document_id:snapshot.document_id})).state,'present','a rejected code is not the requested text')}
    const rerendered=(await host.request({op:'snapshot',...binding})).snapshot;
    assert.match(rerendered.dom_nodes.map(n=>n.text??'').join(' '),/not accepted/,'the page rerendered its form');
    // A card without done_text watches the same element and is not affected by it.
    const plain={op:'popup_region',...binding,card_id:'plain-card',ref:rerendered.dom_nodes.find(n=>n.node_name==='MAIN').node_ref};
    assert.equal((await host.request(plain)).state,'present');
    await click('code');await input({kind:'text',text:'246810'});await input({kind:'key',key:'Enter'});
    let ended;
    for(let i=0;i<40;i++){ended=await host.request({...done,document_id:snapshot.document_id});if(ended.state==='condition_met')break;await delay(100)}
    assert.equal(ended.state,'condition_met');assert.ok(!ended.reason.includes('246810'));
    assert.equal((await host.request({...plain,document_id:snapshot.document_id})).state,'present','without done_text the card stays');
    await assert.rejects(host.request({...done,card_id:'empty',done_text:' '}),/done text must be 1 to 200/);
  } finally {
    await host.stop();if(host.chromium.browserUser)await rm(host.chromium.browserUser.root,{recursive:true,force:true});await rm(root,{recursive:true,force:true});await service.close();
  }
});
