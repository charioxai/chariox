import {createServer,connect} from 'node:net'
import {performance} from 'node:perf_hooks'
import assert from 'node:assert/strict'
import {startShapedProxy} from './shaped-proxy.mjs'

export async function startViewerNetwork(relayOrigin,{httpsOrigins=[]}={}){
 const url=new URL(relayOrigin);assert.equal(url.protocol,'wss:')
 const peers=new Set(),echo=createServer(socket=>{peers.add(socket);socket.on('error',()=>{});socket.pipe(socket);socket.on('close',()=>peers.delete(socket))})
 await new Promise(resolve=>echo.listen(0,'127.0.0.1',resolve))
 const calibrationTarget='127.0.0.1:'+echo.address().port,authority=url.hostname+':'+(url.port||443)
 const webAuthorities=httpsOrigins.map(origin=>{const u=new URL(origin);assert.equal(u.protocol,'https:');assert.equal(u.origin,origin);return u.hostname+':'+(u.port||443)})
 const proxy=await startShapedProxy({targets:[authority,calibrationTarget,...webAuthorities],bindHost:process.env.CTXFIX_BROWSER_NETNS_GATEWAY??"127.0.0.1"})
 let calibrationSocket
 const pending=new Set(),rtts=[];let authenticated=false,disconnects=0,closed=false,closures=[],requestTimings=[],failedRequests=[]
 try{
  calibrationSocket=connect(Number(new URL(proxy.url).port),new URL(proxy.url).hostname);calibrationSocket.on('error',()=>{})
  await new Promise(resolve=>calibrationSocket.once('connect',resolve))
  calibrationSocket.write(`CONNECT ${calibrationTarget} HTTP/1.1\r\nHost: ${calibrationTarget}\r\n\r\n`)
  assert.match((await new Promise(resolve=>calibrationSocket.once('data',resolve))).toString(),/200 Connection Established/)
  const exchange=bytes=>new Promise((resolve,reject)=>{
   let count=0;const start=performance.now(),timer=setTimeout(()=>{calibrationSocket.off('data',receive);reject(Error('Shaper calibration timeout'))},5000)
   const receive=data=>{count+=data.length;if(count>=bytes){clearTimeout(timer);calibrationSocket.off('data',receive);resolve(performance.now()-start)}}
   calibrationSocket.on('data',receive);calibrationSocket.write(Buffer.alloc(bytes,42))
  })
  const echoRtts=[];for(let n=0;n<5;n++)echoRtts.push(await exchange(32))
  const bytes=2*1024*1024,mbps=bytes*8/(await exchange(bytes))/1000
  assert(echoRtts.every(ms=>ms>=60)&&mbps>=7&&mbps<=8.4,'Actual TLS-tunnel leg calibration must enforce60ms and ~8Mbps')
  const calibration={passed:true,echoRttsMs:echoRtts,uplinkObservedMbps:mbps,configuredMbps:8,oneWayDelayMs:30}
  calibrationSocket.destroy();for(const peer of peers)peer.destroy();await new Promise(resolve=>echo.close(resolve))
  return {proxyUrl:proxy.url,calibration,
   attach(page){page.on('requestfailed',request=>{const origin=new URL(request.url()).origin;if(httpsOrigins.includes(origin)||origin===url.origin){failedRequests.push({origin,method:request.method(),failure:request.failure()?.errorText,at:new Date().toISOString()})}});page.on('console',message=>{
    const text=message.text()
    if(!/^\[chariox:kernel-transport\] relay_(request_succeeded|websocket_connected|websocket_closed)/.test(text))return
    const handle=message.args()[1];if(!handle)return
    const work=handle.evaluate(value=>({relayOrigin:typeof value?.relayUrl==='string'?new URL(value.relayUrl).origin:null,requestKind:value?.requestKind,time:performance.now(),rttMs:value?.rttMs,lane:value?.lane,code:value?.code,wasClean:value?.wasClean,reasonClass:typeof (value?.reason??value?.message)==='string'?((value.reason??value.message).toLowerCase().includes('token')?'token':(value.reason??value.message).toLowerCase().includes('timeout')?'timeout':(value.reason??value.message)?'other':'empty'):null})).then(value=>{
     if(closed||(value.relayOrigin!==null&&value.relayOrigin!==url.origin))return
     if(text.includes('relay_request_succeeded')&&Number.isFinite(value.rttMs)){authenticated=true;requestTimings.push(value);if(requestTimings.length>10000)requestTimings.shift();rtts.push(value.rttMs);if(rtts.length>10000)rtts.shift()}
     if(text.includes('relay_websocket_closed')){closures.push({...value,time:new Date().toISOString()});if(value.lane==='control')disconnects++}
    }).catch(()=>{}).finally(()=>pending.delete(work));pending.add(work)
   })},
   async snapshot(){await Promise.allSettled([...pending]);await proxy.sampleStats?.();const connections=proxy.stats.connections.filter(x=>x.authority===authority);return {relayOrigin:url.origin,hostedRelayObserved:connections.length>0,authenticated,shapingObserved:calibration.passed,uplinkMbps:8,rttMs:rtts.length?rtts.reduce((minimum,value)=>Math.min(minimum,value),Infinity):null,rttMeasurement:'authenticated-relay-roundtrip',requestTimings:[...requestTimings],rttSamplesMs:[...rtts],calibration,relayConnections:connections,disconnects,closures:[...closures],shaperBackpressure:proxy.stats.backpressureEvents,shaperQueueDelayMaxMs:proxy.stats.queueDelayMaxMs,shaperFailures:proxy.stats.failureDetails.filter(x=>!(x.stage==='socket_peer_error'&&x.side==='client'&&x.code==='ECONNRESET'&&webAuthorities.includes(x.authority)&&x.queuedUpBytes===0&&x.queuedDownBytes===0)).length,rawTunnelFailures:proxy.stats.failures,shaperFailureDetails:[...proxy.stats.failureDetails],failedRequests:[...failedRequests]}},
   async close(){closed=true;await Promise.allSettled([...pending]);return proxy.close()}}
 }catch(error){calibrationSocket?.destroy();for(const peer of peers)peer.destroy();await new Promise(resolve=>echo.close(resolve));await proxy.close();throw error}
}
