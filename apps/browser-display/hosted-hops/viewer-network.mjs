import {createServer,connect} from 'node:net'
import {performance} from 'node:perf_hooks'
import assert from 'node:assert/strict'
import {startShapedProxy} from './shaped-proxy.mjs'

export async function startViewerNetwork(relayOrigin,{httpsOrigins=[],uplinkMbps=8.75,oneWayDelayMs=30}={}){
 const url=new URL(relayOrigin);assert.equal(url.protocol,'wss:')
 const peers=new Set(),echo=createServer(socket=>{peers.add(socket);socket.pipe(socket);socket.on('close',()=>peers.delete(socket))})
 await new Promise(resolve=>echo.listen(0,'127.0.0.1',resolve))
 const calibrationTarget='127.0.0.1:'+echo.address().port,authority=url.hostname+':'+(url.port||443)
 const webAuthorities=httpsOrigins.map(origin=>{const u=new URL(origin);assert.equal(u.protocol,'https:');assert.equal(u.origin,origin);return u.hostname+':'+(u.port||443)})
 const proxy=await startShapedProxy({targets:[authority,calibrationTarget,...webAuthorities],uplinkMbps,oneWayDelayMs})
 let calibrationSocket
 const pending=new Set(),rtts=[];let authenticated=false,disconnects=0,closed=false
 try{
  calibrationSocket=connect(Number(new URL(proxy.url).port),'127.0.0.1')
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
  assert(echoRtts.every(ms=>ms>=2*oneWayDelayMs)&&mbps>=uplinkMbps*.85&&mbps<=uplinkMbps*1.05,'Actual TLS-tunnel calibration must enforce the configured rate and RTT')
  const calibration={passed:true,echoRttsMs:echoRtts,uplinkObservedMbps:mbps,configuredMbps:uplinkMbps,oneWayDelayMs}
  calibrationSocket.destroy();for(const peer of peers)peer.destroy();await new Promise(resolve=>echo.close(resolve))
  return {proxyUrl:proxy.url,calibration,
   attach(page){page.on('console',message=>{
    const text=message.text()
    if(!/^\[chariox:kernel-transport\] relay_(request_succeeded|websocket_connected|websocket_closed)/.test(text))return
    const handle=message.args()[1];if(!handle)return
    const work=handle.evaluate(value=>({relayOrigin:typeof value?.relayUrl==='string'?new URL(value.relayUrl).origin:null,rttMs:value?.rttMs,lane:value?.lane})).then(value=>{
     if(closed||value.relayOrigin!==url.origin)return
     if(text.includes('relay_request_succeeded')&&Number.isFinite(value.rttMs)){authenticated=true;rtts.push(value.rttMs);if(rtts.length>10000)rtts.shift()}
     if(text.includes('relay_websocket_closed')&&value.lane==='control')disconnects++
    }).catch(()=>{}).finally(()=>pending.delete(work));pending.add(work)
   })},
   async snapshot(){await Promise.allSettled([...pending]);const connections=proxy.stats.connections.filter(x=>x.authority===authority);return {relayOrigin:url.origin,hostedRelayObserved:connections.length>0,authenticated,shapingObserved:calibration.passed,uplinkMbps,rttMs:rtts.length?rtts.reduce((minimum,value)=>Math.min(minimum,value),Infinity):null,rttMeasurement:'authenticated-relay-roundtrip',rttSamplesMs:[...rtts],calibration,relayConnections:connections,disconnects,shaperFailures:proxy.stats.failures,shaperReadBackpressure:proxy.stats.readBackpressure,shaperPeakQueuedBytes:proxy.stats.peakQueuedBytes,shaperFailureReasons:structuredClone(proxy.stats.failureReasons),shaperEndings:structuredClone(proxy.stats.endings)}},
   interrupt(){return proxy.interrupt()},
   resetDisconnects(){disconnects=0},
   async close(){closed=true;await Promise.allSettled([...pending]);return proxy.close()}}
 }catch(error){calibrationSocket?.destroy();for(const peer of peers)peer.destroy();await new Promise(resolve=>echo.close(resolve));await proxy.close();throw error}
}
