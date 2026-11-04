// MP-08 / MP-09 / MP-10 / MP-11: real Chrome input and real HTTP result through the production host.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
import {startChallengeService} from '../../../../scripts/e2e-stack/vault-autonomy/challenge-service.mjs';

test('MP-08/MP-10/MP-11 trusted region input retains rejection and closes only on real acceptance',{timeout:120_000},async()=>{
  const root=await mkdtemp(path.join(tmpdir(),'verification-chrome-')),service=await startChallengeService();
  const host=new KernelBrowserHost(root,{appStorageRoot:path.join(root,'apps')}),scope='agent:verification-chrome';
  try {
    const challenge=service.create(),opened=await host.request({op:'open',url:challenge.url,observed_by:scope});
    const binding={tab_id:opened.tab_id,generation:opened.generation,observed_by:scope};
    const snapshot=(await host.request({op:'snapshot',...binding})).snapshot;
    const main=snapshot.dom_nodes.find(n=>n.node_name==='MAIN');assert.ok(main,'late AX-observed challenge anchor survives the DOM bound');
    assert.ok(snapshot.dom_nodes.length<=5000);
    const command={op:'popup_region',...binding,card_id:'verification-real-chrome',ref:main.node_ref,verification:{challenge_id:challenge.id,url:challenge.url+'/verify',accepted_json_pointer:'/accepted'}};
    const first=await host.request(command);assert.equal(first.state,'present');assert.equal((await(await fetch(challenge.url+'/status')).json()).issued,false);
    const click=async id=>{
      const fresh=(await host.request({op:'snapshot',...binding})).snapshot;
      const node=fresh.dom_nodes.find(n=>n.attributes.id===id);assert.ok(node?.bounds);
      const {x,y,width,height}=node.bounds;
      await host.request({op:'input',...binding,document_id:snapshot.document_id,input:{kind:'click',x:Math.round(x+width/2),y:Math.round(y+height/2)}});
    };
    const input=async value=>host.request({op:'input',...binding,document_id:snapshot.document_id,input:value});
    await click('issue');await delay(100);
    await input({kind:'text',text:'000000'});await input({kind:'key',key:'Enter'});
    let rejected;
    for(let i=0;i<40;i++) {rejected=await host.request({...command,document_id:snapshot.document_id});if(rejected.region?.verification_result?.result==='rejected')break;await delay(100)}
    assert.equal(rejected.state,'present');assert.equal(rejected.region.verification_result?.input_delivered,true);assert.equal(rejected.region.verification_result?.http_status,422);
    await click('code');await input({kind:'key',key:'a',modifiers:2});await input({kind:'text',text:'246810'});await input({kind:'key',key:'Enter'});
    let accepted;
    for(let i=0;i<40;i++) {accepted=await host.request({...command,document_id:snapshot.document_id});if(accepted.state==='condition_met')break;await delay(100)}
    if(accepted.state!=='condition_met')console.log(JSON.stringify({receipt:accepted.region?.verification_result,events:(await(await fetch(challenge.url+'/status')).json()).events}));
    assert.equal(accepted.state,'condition_met');const proof=JSON.parse(accepted.reason);assert.equal(proof.verification_result.result,'accepted');assert.equal(proof.verification_result.http_status,200);
    assert.ok(!JSON.stringify(proof).includes('246810'));assert.ok(!JSON.stringify(proof).includes('000000'));
    const final=await(await fetch(challenge.url+'/status')).json();assert.equal(final.events.filter(e=>e.kind==='challenge_issued').length,1);
    console.log(JSON.stringify({MP:'MP-08 / MP-09 / MP-10 / MP-11',rejected:rejected.region.verification_result,accepted:proof.verification_result}));
  } finally {
    await host.stop();if(host.chromium.browserUser)await rm(host.chromium.browserUser.root,{recursive:true,force:true});await rm(root,{recursive:true,force:true});await service.close();
  }
});
