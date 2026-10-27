import {Worker} from 'node:worker_threads'
// Shape the browser's TLS tunnel as opaque bytes. Never terminate TLS or log
// headers/payloads; the kernel-to-relay leg and host network settings stay intact.
import {createServer} from 'node:http'
import {connect} from 'node:net'
import {performance} from 'node:perf_hooks'
export async function startShapedProxyInProcess({targets,uplinkMbps=8,oneWayDelayMs=30,maxQueuedBytes=8*1024*1024,bindHost="127.0.0.1"}) {
  if(uplinkMbps!==8||oneWayDelayMs<30)throw Error('Real-world shaping requires8Mbit/s uplink and >=30ms each way')
  const allowed=new Set(targets),sockets=new Set(),queues={up:[],down:[]},timers={},stats={uplinkMbps,oneWayDelayMs,connections:[],upBytes:0,downBytes:0,failures:0,failureDetails:[],clientEnds:[],backpressureEvents:[],queueDelayMaxMs:0}
  let freeAt=0,closed=false,queued=0
  // MP-08 / MP-10 / MP-11: drain packets whose delay already elapsed without
  // introducing an extra timer per chunk on the uncapped downstream leg.
  const pump=direction=>{
    if(closed||timers[direction])return
    while(!closed&&!timers[direction]&&queues[direction].length){
      const item=queues[direction][0],wait=item.due-performance.now()
      if(wait>0){timers[direction]=setTimeout(()=>{delete timers[direction];pump(direction)},Math.ceil(wait));return}
      stats.queueDelayMaxMs=Math.max(stats.queueDelayMaxMs,performance.now()-item.due);queues[direction].shift();queued-=item.data.length
      if(item.socket.destroyed||item.socket.writableEnded||!item.socket.writable)continue
      if(item.end){item.socket.end();continue}
      stats[direction+'Bytes']+=item.data.length
      if(item.socket.write(item.data))continue
      const pressure={direction,started:performance.now(),queuedBytes:queued,writableLength:item.socket.writableLength};stats.backpressureEvents.push(pressure);let resumed=false
      const resume=()=>{if(resumed)return;resumed=true;pressure.ms=performance.now()-pressure.started;clearTimeout(timers[direction]);delete timers[direction];item.socket.off('drain',resume);item.socket.off('close',resume);pump(direction)}
      timers[direction]=setTimeout(()=>{stats.failures++;stats.failureDetails.push({stage:'backpressure_timeout',direction,at:new Date().toISOString()});item.socket.destroy();resume()},30000)
      item.socket.once('drain',resume);item.socket.once('close',resume)
      return
    }
  }
  const forward=(direction,target,data)=>{
    const arrived=performance.now()
    for(let offset=0;offset<data.length;offset+=16384){
      const part=data.subarray(offset,Math.min(data.length,offset+16384))
      let due=arrived+oneWayDelayMs
      if(direction==='up'){due=Math.max(due,freeAt)+part.length*8/(uplinkMbps*1000);freeAt=due}
      queued+=part.length
      if(queued>maxQueuedBytes){stats.failures++;stats.failureDetails.push({stage:'queue_capacity',direction,at:new Date().toISOString()});target.destroy();return}
      queues[direction].push({socket:target,data:part,due})
    }
    pump(direction)
  }
  const finish=(direction,target)=>{queues[direction].push({socket:target,data:Buffer.alloc(0),due:performance.now()+oneWayDelayMs,end:true});pump(direction)}
  const server=createServer({allowHalfOpen:true},(_req,res)=>{res.writeHead(405);res.end()})
  server.on('connection',socket=>{sockets.add(socket);socket.on('error',()=>{});socket.on('close',()=>sockets.delete(socket))})
  server.on('clientError',(_error,socket)=>socket.destroy())
  server.on('connect',(req,client,head)=>{
    // req.url is authority only, never a bearer-bearing path/query.
    if(!allowed.has(req.url)){client.end('HTTP/1.1 403 Forbidden\r\n\r\n');return}
    const url=new URL('http://'+req.url),start=performance.now(),remote=connect({port:Number(url.port),host:url.hostname,allowHalfOpen:true})
    const lifecycle={authority:req.url,openedAt:new Date().toISOString(),upBytes:0,downBytes:0}
    sockets.add(client);sockets.add(remote)
    for(const [side,socket] of [['client',client],['remote',remote]]){
      socket.setNoDelay(true)
      socket.on('error',error=>{stats.failures++;stats.failureDetails.push({authority:req.url,code:error.code??error.name,stage:'socket_peer_error',side,readableEnded:socket.readableEnded,writableEnded:socket.writableEnded,writableLength:socket.writableLength,queuedUpBytes:queues.up.filter(x=>x.socket===remote).reduce((n,x)=>n+x.data.length,0),queuedDownBytes:queues.down.filter(x=>x.socket===client).reduce((n,x)=>n+x.data.length,0),peerReadableEnded:(side==='client'?remote:client).readableEnded,peerDestroyed:(side==='client'?remote:client).destroyed,at:new Date().toISOString(),lifecycle:{...lifecycle,ageMs:performance.now()-start}});client.destroy();remote.destroy()})
      socket.on('close',()=>sockets.delete(socket))
    }
    remote.once('connect',()=>{
      stats.connections.push({authority:req.url,remoteAddress:remote.remoteAddress,tcpConnectMs:performance.now()-start})
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n')
      if(head.length)forward('up',remote,head)
      client.on('data',data=>{lifecycle.upBytes+=data.length;forward('up',remote,data)});remote.on('data',data=>{lifecycle.downBytes+=data.length;forward('down',client,data)})
      // Chromium closes the TLS tunnel as a duplex connection. Retaining the
      // upstream half after that FIN lets delayed HTTPS response bytes reach a
      // browser socket which no longer accepts them. Close this exact tunnel;
      // active WebSocket closures remain independently visible to the drill.
      client.once('end',()=>{
        stats.clientEnds.push({authority:req.url,at:new Date().toISOString()})
        remote.destroy();client.destroy()
      })
      client.once('close',()=>remote.destroy())
      remote.once('end',()=>finish('down',client))
    })
  })
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,bindHost,resolve)})
  return {url:'http://'+bindHost+':'+server.address().port,stats,
    async close(){closed=true;for(const timer of Object.values(timers))clearTimeout(timer);for(const socket of sockets)socket.destroy();queues.up.length=queues.down.length=0;await new Promise(resolve=>server.close(resolve));return stats}}
}

// MP-08 / MP-10 / MP-11: keep network scheduling independent of driver JSON/CDP/evidence work.
export async function startShapedProxy(options){
 const worker=new Worker(new URL('./shaped-proxy-worker.mjs',import.meta.url),{workerData:options});
 let nextId=0,stats={},pending=new Map();
 const ready=new Promise((resolve,reject)=>{
  worker.on('message',m=>{if(m.kind==='ready'){stats=m.stats;resolve(m.url)}else if(m.kind==='stats'){stats=m.stats;pending.get(m.id)?.resolve(stats);pending.delete(m.id)}else if(m.kind==='closed'){stats=m.stats;pending.get(m.id)?.resolve(stats);pending.delete(m.id)}});
  worker.on('error',e=>{reject(e);for(const p of pending.values())p.reject(e);pending.clear()});
 });
 const url=await ready;
 const request=kind=>new Promise((resolve,reject)=>{const id=++nextId;pending.set(id,{resolve,reject});worker.postMessage({kind,id})});
 return {url,get stats(){return stats},async sampleStats(){return request('stats')},async close(){try{return await request('close')}finally{await worker.terminate()}}};
}
