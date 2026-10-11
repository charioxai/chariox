// MP-08/MP-10/MP-11: pin the authorized private validation fixture origin.
import {readFile} from 'node:fs/promises';
const origin=process.env.AB_ORIGIN;
const u=new URL(origin);
if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||u.pathname!=='/')throw Error('MP-11: public HTTPS origin required');
const apiOrigin=process.env.AB_API_ORIGIN??origin;
const api=new URL(apiOrigin);
if(api.protocol!=='https:'||api.username||api.password||api.search||api.hash||api.pathname!=='/')throw Error('MP-11: public HTTPS API origin required');
const request=(path,options={})=>fetch(apiOrigin+path,{...options,redirect:'manual',signal:AbortSignal.timeout(15000)});
const artifacts=JSON.parse(await readFile(new URL('./ARTIFACTS.json',import.meta.url),'utf8'));
const qualification=artifacts.validation_qualification;
if(qualification?.status!=='PASS'||qualification.origin!==u.origin||api.origin!==u.origin)throw Error('MP-08/MP-10: exact validation viewer/runtime qualification required');
if(!/^pr[0-9]+-vidlat[a-z0-9-]*\.val\.51-255-87-178\.sslip\.io$/.test(u.hostname))throw Error('MP-11: owned private validation origin required');
if(qualification.fixture_email!==u.hostname.split('.')[0]+'@chariox.invalid'||qualification.cloud!==artifacts.cloud||qualification.oss!==artifacts.oss)throw Error('MP-11: fixture/source binding mismatch');
const fixture=await request('/validation/ready');
const ready=await fixture.json().catch(()=>null);
if(!fixture.ok||ready?.ok!==true||ready.checks?.fixtureAuth?.ok!==true)throw Error('MP-08/MP-10: private validation fixture unavailable');
// Poll requires a device code. Invalid input validates machine admission without
// creating a pending challenge; start permits omitted client fields.
const device=await request('/auth/device/poll',{method:'POST',headers:{'content-type':'application/json'},body:'{}'});
if(device.status!==400)throw Error('MP-08/MP-10: product device-poll endpoint is gated/unavailable');
const body=await device.json().catch(()=>null);
if(!body?.error)throw Error('MP-11: expected product device validation response required');
const dashboard=await request('/dashboard');
if(!dashboard.ok)throw Error('MP-08/MP-10: one-step fixture browser access unavailable');
const session=await request('/auth/session');
const identity=await session.json().catch(()=>null);
if(!session.ok||identity?.authenticated!==true||identity.identity?.email!==qualification.fixture_email)throw Error('MP-11: disposable fixture browser identity mismatch');
console.log('MP-08/MP-10/MP-11: qualified private fixture and product machine access passed');
