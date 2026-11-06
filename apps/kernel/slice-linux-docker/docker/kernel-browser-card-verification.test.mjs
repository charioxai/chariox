// MP-08 / MP-09 / MP-10 / MP-11: positive acceptance and observation ownership.
import test from 'node:test';
import assert from 'node:assert/strict';
import {setTimeout as delay} from 'node:timers/promises';
import {BrowserCardVerification,validateCardVerification} from './kernel-browser-card-verification.mjs';
const config={challenge_id:'challenge-1',url:'https://example.com/verify',accepted_json_pointer:'/sudo'};
async function fixture(run) {
  const listeners=new Set(),bodies=new Map(),frame={id:'frame',loaderId:'document'};
  const connection={subscribe(fn){listeners.add(fn);return()=>listeners.delete(fn)},async send(method,args){
    if(method==='Page.getFrameTree')return{frameTree:{frame:{...frame}}};
    if(method==='Runtime.callFunctionOn')return{result:{objectId:'cleanup'}};
    if(method==='Network.getResponseBody')return bodies.get(args.requestId) ?? {body:'{"sudo":true,"token":"not-in-receipt"}'};
    return{};
  }};
  const watch=await BrowserCardVerification.create(connection,'session',{...frame},config);
  await watch.bindInput('anchor',7);
  let inputSequence=0;
  const event=(method,params,sessionId='session')=>{for(const fn of listeners)fn({sessionId,method,params})};
  const input=()=>event('Runtime.bindingCalled',{name:watch.bindingName,executionContextId:7,payload:JSON.stringify({sequence:++inputSequence,value:'code-secret'})});
  const request=(id,changes={})=>{input();event('Network.requestWillBeSent',{requestId:id,frameId:'frame',loaderId:'document',hasUserGesture:true,request:{method:'POST',url:config.url,headers:{authorization:'secret'},postData:'{"code":"code-secret"}',...changes}})};
  const response=async(id,status=200)=>{event('Network.responseReceived',{requestId:id,response:{url:config.url,status,headers:{'set-cookie':'private'}}});event('Network.loadingFinished',{requestId:id,encodedDataLength:100});await delay(0)};
  try {await run({watch,event,request,response,bodies,frame,listeners,connection,input})} finally {watch.close()}
}
for(const body of ['{"sudo":false}','{"sudo":"true"}','{"sudo":1}','{}','not JSON'])test('MP-11 nonpositive body cannot verify: '+body,()=>fixture(async({watch,request,response,bodies})=>{
  bodies.set('r',{body});request('r');await response('r');assert.equal(watch.receipt.result,'rejected');assert.ok(!JSON.stringify(watch.receipt).includes('secret'));
}));
test('MP-11 HTTP failure cannot be accepted by a positive-looking body',()=>fixture(async({watch,request,response})=>{request('r');await response('r',403);assert.equal(watch.receipt.result,'rejected');assert.equal(watch.receipt.http_status,403)}));
test('MP-11 a lost network request has delivery but no acceptance',()=>fixture(async({watch,request,event})=>{request('r');event('Network.loadingFailed',{requestId:'r',errorText:'private-network-detail'});assert.equal(watch.receipt.result,'failed');assert.ok(!JSON.stringify(watch.receipt).includes('private-network-detail'))}));
test('MP-11 an older response cannot accept the current attempt',()=>fixture(async({watch,request,response,bodies})=>{
  request('old');request('current');await response('old');assert.equal(watch.receipt.request_id,'current');assert.equal(watch.receipt.result,'delivered');bodies.set('current',{body:'{"sudo":false}'});await response('current');assert.equal(watch.receipt.result,'rejected');
}));
test('MP-11 navigation during response read cannot verify the replacement document',()=>fixture(async({watch,request,response,frame})=>{request('r');frame.loaderId='replacement';await response('r');assert.equal(watch.receipt.result,'failed')}));
test('MP-11 redirects do not change the declared verification endpoint',()=>fixture(async({watch,request,event,response})=>{request('r');event('Network.requestWillBeSent',{requestId:'r',redirectResponse:{status:302},request:{method:'POST',url:'https://foreign.example/verify'}});await response('r');assert.equal(watch.receipt.result,'failed')}));
test('MP-11 oversized verification bodies are not read',()=>fixture(async({watch,request,event,connection})=>{
  request('r');event('Network.responseReceived',{requestId:'r',response:{url:config.url,status:200}});connection.send=async()=>{throw Error('must not read')};event('Network.loadingFinished',{requestId:'r',encodedDataLength:65537});await delay(0);assert.equal(watch.receipt.result,'failed');
}));
test('MP-11 base64 JSON yields only the fixed accepted receipt',()=>fixture(async({watch,request,response,bodies})=>{bodies.set('r',{body:Buffer.from('{"sudo":true,"token":"not-in-receipt"}').toString('base64'),base64Encoded:true});request('r');await response('r');assert.deepEqual(watch.receipt,{challenge_id:'challenge-1',request_id:'r',input_delivered:true,http_status:200,result:'accepted'})}));
test('MP-11 an expired card detaches the listener and cannot complete later',()=>fixture(async({watch,request,response,listeners})=>{request('r');watch.close();assert.equal(listeners.size,0);await response('r');assert.equal(watch.receipt.result,'delivered')}));
test('MP-11 verification config requires an exact same-origin endpoint and boolean JSON pointer',()=>{
  assert.deepEqual(validateCardVerification(config,'https://example.com'),config);
  for(const changes of [{url:'https://foreign.example/verify'},{url:config.url+'?code=private'},{url:'https://user:pass@example.com/verify'},{accepted_json_pointer:'sudo'},{accepted_json_pointer:'/~2'},{challenge_id:''},{body:'private'}])assert.throws(()=>validateCardVerification({...config,...changes},'https://example.com'));
});
test('MP-08/MP-11 an unrelated same-frame POST cannot supply a challenge acceptance',()=>fixture(async({watch,event,response})=>{
  event('Network.requestWillBeSent',{requestId:'background',frameId:'frame',loaderId:'document',hasUserGesture:false,request:{method:'POST',url:config.url}});
  await response('background');assert.equal(watch.receipt,null,'no input delivery occurred inside this card');
}));
test('MP-08/MP-11 a different code in a gesture POST cannot accept the bound input',()=>fixture(async({watch,event,response,input})=>{
  input();event('Network.requestWillBeSent',{requestId:'different-code',frameId:'frame',loaderId:'document',hasUserGesture:true,request:{method:'POST',url:config.url,postData:'{"code":"different-submission"}'}});
  await response('different-code');assert.equal(watch.receipt,null,'the body must carry the code delivered to this card field');
}));

for(const [contentType,postData] of [
  ['application/x-www-form-urlencoded;charset=UTF-8','code=code%2Dsecret'],
  ['Application/X-Www-Form-Urlencoded; charset=utf-8','unrelated=x&code=code-secret'],
])test('MP-08/MP-10/MP-11 form encoding yields a positive receipt: '+contentType,()=>fixture(async({watch,request,response})=>{
  request('form',{headers:{'Content-Type':contentType},postData});await response('form');
  assert.equal(watch.receipt?.result,'accepted');assert.ok(!JSON.stringify(watch.receipt).includes('code-secret'));
}));
for(const postData of ['code=different','code-secret=different','code=code%252Dsecret'])test('MP-11 URL-encoded correlation requires the exact field value: '+postData,()=>fixture(async({watch,request,response})=>{
  request('form',{headers:{'content-type':'application/x-www-form-urlencoded'},postData});await response('form');assert.equal(watch.receipt,null);
}));

for(const [disposition,value,accepted] of [
  ['name="code"','code-secret',true],
  ['name="code"','different',false],
  ['name="code-secret"','different',false],
  ['name="file"; filename="code.txt"','code-secret',false],
])test('MP-11 multipart correlates text field values only: '+disposition+value,()=>fixture(async({watch,request,response})=>{
  const postData='--card-boundary\r\nContent-Disposition: form-data; '+disposition+'\r\n\r\n'+value+'\r\n--card-boundary--\r\n';
  request('form',{headers:{'content-type':'multipart/form-data; boundary="card-boundary"'},postData});await response('form');
  assert.equal(watch.receipt?.result==='accepted',accepted);
}));
