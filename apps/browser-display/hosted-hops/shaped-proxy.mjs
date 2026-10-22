// MP-08/MP-10/MP-11: shape opaque TLS bytes without terminating TLS or logging
// payloads. Stop consuming source streams when the bounded queue is full.
import {createServer} from 'node:http'
import {connect} from 'node:net'
import {performance} from 'node:perf_hooks'

export async function startShapedProxy({targets,uplinkMbps=8,oneWayDelayMs=30,maxQueuedBytes=8*1024*1024}) {
  if(!Number.isFinite(uplinkMbps)||uplinkMbps<=0||!Number.isFinite(oneWayDelayMs)||oneWayDelayMs<30||!Number.isSafeInteger(maxQueuedBytes)||maxQueuedBytes<2)
    throw Error('Shaping requires a positive Mbps rate, >=30ms each way and a queue of at least two bytes')
  const allowed=new Set(targets),endings=new Set(),sockets=new Set(),queues={up:[],down:[]},readers={up:new Set(),down:new Set()},queued={up:0,down:0},timers={}
  // Reserve capacity for both directions so a full uplink never stops replies.
  const limits={up:Math.floor(maxQueuedBytes/2),down:Math.ceil(maxQueuedBytes/2)}
  const stats={uplinkMbps,oneWayDelayMs,connections:[],upBytes:0,downBytes:0,failures:0,readBackpressure:0,peakQueuedBytes:0,failureReasons:{backpressureTimeout:0,socketErrors:{}},endings:[]}
  let freeAt=0,closed=false
  const pump=direction=>{
    if(closed||timers[direction]||!queues[direction].length)return
    const item=queues[direction][0],wait=Math.max(0,item.due-performance.now())
    timers[direction]=setTimeout(()=>{
      delete timers[direction]
      // Timers can wake before a monotonic deadline; do not understate RTT.
      if(performance.now()<item.due){pump(direction);return}
      queues[direction].shift();queued[direction]-=item.data.length
      if(item.socket.destroyed){consume(direction);pump(direction);return}
      if(item.end){item.socket.end();consume(direction);pump(direction);return}
      stats[direction+'Bytes']+=item.data.length
      if(item.socket.write(item.data)){consume(direction);pump(direction);return}
      // Bound the destination's writable buffer as well as the delay queue.
      let resumed=false
      const resume=()=>{if(resumed)return;resumed=true;clearTimeout(timers[direction]);delete timers[direction];item.socket.off('drain',resume);item.socket.off('close',resume);consume(direction);pump(direction)}
      timers[direction]=setTimeout(()=>{stats.failures++;stats.failureReasons.backpressureTimeout++;item.socket.destroy();resume()},30000)
      item.socket.once('drain',resume);item.socket.once('close',resume)
      consume(direction)
    },Math.ceil(wait))
  }
  const consume=direction=>{
    if(closed)return
    // Socket readable mode leaves unread bytes in Node's bounded read buffer,
    // which stops TCP reads at its high-water mark. No overflow is discarded.
    for(const reader of readers[direction]){
      while(!reader.source.destroyed&&reader.source.readableLength){
        const capacity=limits[direction]-queued[direction]
        if(!capacity){if(!reader.blocked){reader.blocked=true;stats.readBackpressure++}break}
        reader.blocked=false
        const data=reader.source.read(Math.min(16384,capacity,reader.source.readableLength))
        if(!data)break
        let due=performance.now()+oneWayDelayMs
        if(direction==='up'){due=Math.max(due,freeAt)+data.length*8/(uplinkMbps*1000);freeAt=due}
        queued[direction]+=data.length
        stats.peakQueuedBytes=Math.max(stats.peakQueuedBytes,queued.up+queued.down)
        queues[direction].push({socket:reader.target,data,due})
      }
      if(!reader.source.readableLength)reader.source.read(0)
    }
    pump(direction)
  }
  const forward=(direction,source,target)=>{
    const reader={source,target,blocked:false};readers[direction].add(reader)
    source.on('readable',()=>consume(direction))
    source.once('end',()=>{readers[direction].delete(reader);queues[direction].push({socket:target,data:Buffer.alloc(0),due:performance.now()+oneWayDelayMs,end:true});pump(direction)})
    source.once('close',()=>readers[direction].delete(reader))
    consume(direction)
  }
  const server=createServer({allowHalfOpen:true},(_req,res)=>{res.writeHead(405);res.end()})
  server.on('connect',(req,client,head)=>{
    // req.url is authority only, never a bearer-bearing path/query.
    if(!allowed.has(req.url)){
      client.on('error',()=>client.destroy());client.end('HTTP/1.1 403 Forbidden\r\n\r\n');return
    }
    client.pause()
    const url=new URL('http://'+req.url),start=performance.now(),remote=connect({port:Number(url.port),host:url.hostname,allowHalfOpen:true})
    sockets.add(client);sockets.add(remote)
    // Which side ended this tunnel first, so a 1006 close can be attributed to the browser, the shaper or the server.
    const ending={authority:req.url,openedAt:Date.now()},first=(side,how)=>{if(ending.first)return;Object.assign(ending,{first:side,how,at:Date.now(),queuedUp:queued.up,queuedDown:queued.down});stats.endings.push(ending)}
    endings.add(first)
    for(const [side,socket] of [['client',client],['remote',remote]]){
      socket.once('close',()=>endings.delete(first));socket.once('end',()=>first(side,'end'));socket.once('error',error=>first(side,'error:'+String(error.code||'UNKNOWN')));socket.once('close',()=>first(side,'close'))
    }
    for(const socket of [client,remote]){
      socket.setNoDelay(true)
      socket.on('error',error=>{if(!closed){stats.failures++;const code=String(error.code||'UNKNOWN');stats.failureReasons.socketErrors[code]=(stats.failureReasons.socketErrors[code]||0)+1}client.destroy();remote.destroy()})
      socket.on('close',()=>sockets.delete(socket))
    }
    remote.once('connect',()=>{
      stats.connections.push({authority:req.url,remoteAddress:remote.remoteAddress,tcpConnectMs:performance.now()-start})
      client.write('HTTP/1.1 200 Connection Established\r\n\r\n')
      if(head.length)client.unshift(head)
      forward('up',client,remote);forward('down',remote,client)
    })
  })
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve)})
  return {url:'http://127.0.0.1:'+server.address().port,stats,
    interrupt(){for(const first of endings)first('shaper','interrupt');for(const socket of sockets)socket.destroy();for(const timer of Object.values(timers))clearTimeout(timer);for(const key of Object.keys(timers))delete timers[key];queues.up.length=queues.down.length=0;queued.up=queued.down=0;readers.up.clear();readers.down.clear();freeAt=performance.now();},
    async close(){for(const first of endings)first('shaper','close');closed=true;for(const timer of Object.values(timers))clearTimeout(timer);for(const socket of sockets)socket.destroy();queues.up.length=queues.down.length=0;readers.up.clear();readers.down.clear();await new Promise(resolve=>server.close(resolve));return stats}}
}
