// MD-DISPLAY-03: run the existing installed streamer; never read/export auth state.
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createInterface } from 'node:readline';
import { createServer } from 'node:net';
import { pause, until } from './runtime.mjs';
const execute=promisify(execFile);
const docker=async(...args)=>(await execute('docker',args,{maxBuffer:1024*1024,timeout:60000})).stdout.trim();
export async function baseline(chromium,origin,output) {
  const name=`md-display-selkies-${process.pid}-${Date.now()}`;
  const image=process.env.MD_BASELINE_IMAGE||'chariox-local-headed:f1c402b82aa0053bb69f0f8fffe04b06e02a5c73';
  const imageMetadata=JSON.parse(await docker('image','inspect',image,'--format','{{json .}}'));
  const metadata={name,image_tag:image,image_id:imageMetadata.Id,repo_digests:imageMetadata.RepoDigests,labels:imageMetadata.Config.Labels,host_limits:{cpus:2,memory_bytes:3*1024**3,swap_extra_bytes:0,pids:512},source_attribution:'installed image labels, independent of harness G2 base',sandbox:false,sandbox_reason:'credential-free baseline Chrome only; Docker default seccomp prevents host namespace sandbox; this is not a production sandbox test'};
  let created=false,browser,stream,renew,chrome,xserver;
  const close=async()=>{
    clearInterval(renew);stream?.stdin.end();
    if(stream)await Promise.race([new Promise(r=>stream.once('exit',r)),pause(5000)]);
    if(browser)try{await browser.close()}catch{}
    if(created){
      const labels=JSON.parse(await docker('inspect',name,'--format','{{json .Config.Labels}}'));
      if(labels['chariox.lane']!=='display'||labels['chariox.task']!=='md-display')throw Error('MD-DISPLAY container ownership mismatch');
      await docker('rm','-f',name);created=false;
    }
    chrome?.kill();xserver?.kill();
  };
  try{
    await docker('run','-d','--name',name,'--label','chariox.lane=display','--label','chariox.task=md-display','--network=host','--memory=3g','--memory-swap=3g','--cpus=2','--pids-limit=512','--shm-size=512m','--entrypoint','/bin/bash',image,'-c','sleep 1800');created=true;
    await docker('exec',name,'sh','-c','mkdir -m 700 /tmp/md-display-runtime /tmp/md-display-profile');
    xserver=spawn('docker',['exec',name,'Xvfb',':99','-screen','0','1920x1200x24','-nolisten','tcp','-ac'],{stdio:'ignore'});await pause(300);
    chrome=spawn('docker',['exec','-e','DISPLAY=:99',name,'/usr/bin/chromium','--no-sandbox','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0','--user-data-dir=/tmp/md-display-profile','--no-first-run','--disable-background-networking','--disable-component-update','--disable-sync','--disable-dev-shm-usage','--force-device-scale-factor=2','--window-size=1920,1200','--window-position=0,0','--kiosk',`--app=${origin}/docs`],{stdio:'ignore'});
    const port=await until(async()=>{try{return Number((await docker('exec',name,'python3','-c','from pathlib import Path; print(Path("/tmp/md-display-profile/DevToolsActivePort").read_text().splitlines()[0])')).trim())}catch{return null}},'baseline Chrome',20000);
    browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    metadata.browser_version=browser.version();
    metadata.streamer_source=await docker('exec',name,'python3','-c','import hashlib; from pathlib import Path; print(hashlib.sha256(Path("/opt/chariox-slice/slice-selkies.py").read_bytes()).hexdigest())');
    const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const displayPort=reservation.address().port;await new Promise(r=>reservation.close(r));metadata.loopback_port=displayPort;
    await docker('exec','-e','DISPLAY=:99','-e','XDG_RUNTIME_DIR=/tmp/md-display-runtime','-e',`CHARIOX_SLICE_NOVNC_PORT=${displayPort}`,name,'/opt/chariox-selkies/bin/python','/opt/chariox-slice/slice-selkies.py','start');
    return {browser,scratch:null,metadata,child:null,close,
      async sample(){return JSON.parse(await docker('stats','--no-stream','--format','{{json .}}',name))},
      async start(send,onFrame){
        stream=spawn('docker',['exec','-i','-e','XDG_RUNTIME_DIR=/tmp/md-display-runtime',name,'/opt/chariox-selkies/bin/python','/opt/chariox-slice/slice-selkies-stream.py','--lease-ms','60000'],{stdio:['pipe','pipe','ignore']});
        let ready=false,configured=false;const packetDimensions=[];
        createInterface({input:stream.stdout}).on('line',line=>{
          const record=JSON.parse(line);if(record.kind==='ready'){ready=true;return}if(record.kind!=='binary')return;
          const bytes=Buffer.from(record.data_base64,'base64');if(bytes[0]!==4||bytes.length<11)return;
          const key=bytes[1]===1,id=bytes.readUInt16BE(2),y=bytes.readUInt16BE(4),width=bytes.readUInt16BE(6),height=bytes.readUInt16BE(8),payload=bytes.subarray(10);
          onFrame(bytes.length);packetDimensions.push([width,height,y]);
          if(y!==0)throw Error('MD-DISPLAY baseline unexpectedly striped');
          let config;
          if(!configured&&key){
            let codec='avc1.640033';
            for(let i=0;i<payload.length-7;i++)if(payload[i]===0&&payload[i+1]===0&&payload[i+2]===1&&(payload[i+3]&31)===7){codec='avc1.'+payload.subarray(i+4,i+7).toString('hex').toUpperCase();break}
            config={codec,codedWidth:width,codedHeight:height,optimizeForLatency:true};configured=true;metadata.decoder_config=config;
          }
          if(configured)send({kind:'chunk',type:key?'key':'delta',timestamp:Math.round(performance.now()*1000),config},payload);
          stream.stdin.write(JSON.stringify({kind:'control',text:`CLIENT_FRAME_ACK ${id}`})+'\n');
        });
        await until(()=>ready,'read-only installed Selkies adapter ready',15000);
        renew=setInterval(()=>stream.stdin.write('{"kind":"renew"}\n'),15000);
        metadata.packet_dimensions=packetDimensions;
      },
      async stop(){clearInterval(renew);stream?.stdin.end();if(stream)await Promise.race([new Promise(r=>stream.once('exit',r)),pause(5000)]);stream=null},
    };
  }catch(e){await close();throw e}
}
