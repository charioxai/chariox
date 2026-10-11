// MP-08/MP-10/MP-11: verify machine access and reviewed staging compatibility.
import {readFile} from 'node:fs/promises';
const origin=process.env.AB_ORIGIN;
const u=new URL(origin);
if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||u.pathname!=='/')throw Error('MP-11: public HTTPS origin required');
const apiOrigin=process.env.AB_API_ORIGIN??origin;
const api=new URL(apiOrigin);
if(api.protocol!=='https:'||api.username||api.password||api.search||api.hash||api.pathname!=='/')throw Error('MP-11: public HTTPS API origin required');
const request=(path,options={})=>fetch(apiOrigin+path,{...options,redirect:'manual',signal:AbortSignal.timeout(15000)});
const fixture=await request('/validation/ready');
if(fixture.ok)throw Error('MP-08/MP-10: owner mode requires staging product identity, not validation fixture login');
// Poll requires a device code. Invalid input validates machine admission without
// creating a pending challenge; start permits omitted client fields.
const device=await request('/auth/device/poll',{method:'POST',headers:{'content-type':'application/json'},body:'{}'});
if(device.status!==400)throw Error('MP-08/MP-10: product device-poll endpoint is gated/unavailable');
const body=await device.json().catch(()=>null);
if(!body?.error)throw Error('MP-11: expected product device validation response required');
const artifacts=JSON.parse(await readFile(new URL('./ARTIFACTS.json',import.meta.url),'utf8'));
const qualification=artifacts.staging_qualification;
if(qualification?.status!=='PASS'||qualification.origin!==u.origin)throw Error('MP-08/MP-10: staging video viewer/runtime compatibility has not passed; coordinator action is recorded in OPERATOR.md');
console.log('MP-08/MP-10/MP-11: product machine access and reviewed staging compatibility passed');
