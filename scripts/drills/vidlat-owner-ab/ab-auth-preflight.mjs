// MP-08/MP-10/MP-11: owner use must enter through normal product login.
const origin=process.env.AB_ORIGIN;
const u=new URL(origin);
if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash)throw Error('MP-11: public HTTPS origin required');
const fixture=await fetch(origin+'/validation/ready',{redirect:'manual',signal:AbortSignal.timeout(15000)});
if(fixture.ok)throw Error('MP-08/MP-10: fixture-only auth is unsuitable for owner sign-in; coordinator must configure normal Auth0/WorkOS');
const login=await fetch(origin+'/auth/login?returnTo=%2Fwaiting-room',{redirect:'manual',signal:AbortSignal.timeout(15000)});
if(![302,303].includes(login.status))throw Error('MP-08/MP-10: normal product login redirect unavailable');
const location=new URL(login.headers.get('location')??'',origin);
if(location.protocol!=='https:'||location.origin===u.origin)throw Error('MP-11: external normal-auth login required');
console.log('MP-08/MP-10/MP-11: normal product sign-in redirect available');
