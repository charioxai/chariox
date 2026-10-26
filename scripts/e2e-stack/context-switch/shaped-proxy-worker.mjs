// MP-08 / MP-10 / MP-11: opaque TLS network scheduling on a dedicated event loop.
import {parentPort,workerData} from 'node:worker_threads';
import {startShapedProxyInProcess} from './shaped-proxy.mjs';
const proxy=await startShapedProxyInProcess(workerData);
parentPort.postMessage({kind:'ready',url:proxy.url,stats:proxy.stats});
parentPort.on('message',async m=>{if(m.kind==='stats')parentPort.postMessage({kind:'stats',id:m.id,stats:proxy.stats});else if(m.kind==='close')parentPort.postMessage({kind:'closed',id:m.id,stats:await proxy.close()})});
