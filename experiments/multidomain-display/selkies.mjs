// MD-DISPLAY-03: run the existing installed streamer; never read/export auth state.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import {readPackets} from './selkies-packets.mjs';
import {launchOwned,checkChild,signalChild,waitChild} from './owned-process.mjs';
import { createServer } from 'node:net';
import { pause, until, failRun } from './runtime.mjs';
const execute=promisify(execFile);
const docker=async(...args)=>(await execute('docker',args,{maxBuffer:1024*1024,timeout:60000})).stdout.trim();
export async function baseline(chromium,origin,output) {
  const name=`md-display-selkies-${process.pid}-${Date.now()}`;
  const image=process.env.MD_BASELINE_IMAGE||'chariox-local-headed:f1c402b82aa0053bb69f0f8fffe04b06e02a5c73';
  const imageMetadata=JSON.parse(await docker('image','inspect',image,'--format','{{json .}}'));
  const metadata={name,image_tag:image,image_id:imageMetadata.Id,repo_digests:imageMetadata.RepoDigests,labels:imageMetadata.Config.Labels,host_limits:{cpus:2,memory_bytes:3*1024**3,swap_extra_bytes:0,pids:512},source_attribution:'installed image labels, independent of harness G2 base',sandbox:false,sandbox_reason:'credential-free baseline Chrome only; Docker default seccomp prevents host namespace sandbox; this is not a production sandbox test'};
  let created=false,browser,stream,renew,chrome,xserver,packets;
  const close=async()=>{
    clearInterval(renew);packets?.close();stream?.stdin.end();
    if(stream)await Promise.race([waitChild(stream),pause(5000)]);
    if(browser)try{await browser.close()}catch{}
    if(created){
      const labels=JSON.parse(await docker('inspect',name,'--format','{{json .Config.Labels}}'));
      if(labels['chariox.lane']!=='display'||labels['chariox.task']!=='md-display')throw Error('MD-DISPLAY container ownership mismatch');
      await docker('rm','-f',name);created=false;
    }
    await signalChild(chrome);await signalChild(xserver);
  };
  try{
    await docker('run','-d','--name',name,'--label','chariox.lane=display','--label','chariox.task=md-display','--network=host','--memory=3g','--memory-swap=3g','--cpus=2','--pids-limit=512','--shm-size=512m','--entrypoint','/bin/bash',image,'-c','sleep 1800');created=true;
    await docker('exec',name,'sh','-c','mkdir -m 700 /tmp/md-display-runtime /tmp/md-display-profile');
    xserver=await launchOwned('docker',['exec',name,'Xvfb',':99','-screen','0','1920x1200x24','-nolisten','tcp','-ac'],{stdio:'ignore'});await pause(300);checkChild(xserver,'baseline Xvfb');
    chrome=await launchOwned('docker',['exec','-e','DISPLAY=:99',name,'/usr/bin/chromium','--no-sandbox','--remote-debugging-address=127.0.0.1','--remote-debugging-port=0','--user-data-dir=/tmp/md-display-profile','--no-first-run','--disable-background-networking','--disable-component-update','--disable-sync','--disable-dev-shm-usage','--force-device-scale-factor=2','--window-size=1920,1200','--window-position=0,0','--kiosk',`--app=${origin}/docs`],{stdio:'ignore'});
    const port=await until(async()=>{checkChild(chrome,'baseline Chrome');try{return Number((await docker('exec',name,'python3','-c','from pathlib import Path; print(Path("/tmp/md-display-profile/DevToolsActivePort").read_text().splitlines()[0])')).trim())}catch{return null}},'baseline Chrome',20000);
    browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    metadata.browser_version=browser.version();
    metadata.streamer_source=await docker('exec',name,'python3','-c','import hashlib; from pathlib import Path; print(hashlib.sha256(Path("/opt/chariox-slice/slice-selkies.py").read_bytes()).hexdigest())');
    const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const displayPort=reservation.address().port;await new Promise(r=>reservation.close(r));metadata.loopback_port=displayPort;
    const settings=process.env.MD_BASELINE_RATE==='cbr'?{rate_control_mode:'cbr',video_bitrate:Number(process.env.MD_BITRATE||8000000)/1000,use_paint_over_quality:process.env.MD_BASELINE_PAINT==='1'}:{};
    metadata.requested_settings=settings;
    metadata.default_settings={rate_control_mode:'crf',video_crf:25,video_bitrate_kbps:8000,use_paint_over_quality:true,video_paintover_crf:18,video_fullcolor:false,framerate:30};
    metadata.settings_source='installed selkies/settings.py; same CLI/env parser selections recorded below; no live private config read';
    const settingsEnv=Object.entries(settings).flatMap(([key,value])=>['-e',`SELKIES_${key.toUpperCase()}=${value}`]);
    await docker('exec',...settingsEnv,'-e','DISPLAY=:99','-e','XDG_RUNTIME_DIR=/tmp/md-display-runtime','-e',`CHARIOX_SLICE_NOVNC_PORT=${displayPort}`,name,'/opt/chariox-selkies/bin/python','/opt/chariox-slice/slice-selkies.py','start');
    metadata.resolved_parser_settings=JSON.parse(await docker('exec',...settingsEnv,name,'/opt/chariox-selkies/bin/python','-c',"from selkies.settings import AppSettings,SETTING_DEFINITIONS,software_h264_encoder; import json,sys; sys.argv=['probe','--mode=websockets','--encoder=h264enc','--use-cpu=true|locked','--framerate=30']; s=AppSettings(SETTING_DEFINITIONS); s.resolve_rate_control_default(); keys=['rate_control_mode','video_bitrate','video_crf','use_paint_over_quality','video_paintover_crf','video_fullcolor','framerate']; print(json.dumps({**{k:getattr(s,k,None) for k in keys},'software_h264_encoder':software_h264_encoder(),'initial_range_values':{d['name']:d.get('meta',{}).get('default_value') for d in SETTING_DEFINITIONS if d['name'] in keys and d.get('type')=='range'}}))"));
    return {browser,scratch:null,metadata,child:null,close,
      async sample(){return JSON.parse(await docker('stats','--no-stream','--format','{{json .}}',name))},
      async start(send,onFrame){
        stream=await launchOwned('docker',['exec','-i','-e','XDG_RUNTIME_DIR=/tmp/md-display-runtime',name,'/opt/chariox-selkies/bin/python','/opt/chariox-slice/slice-selkies-stream.py','--lease-ms','60000'],{stdio:['pipe','pipe','ignore']});
        packets=readPackets(stream,(meta,bytes)=>{if(meta.config)metadata.decoder_config=meta.config;send(meta,bytes)},onFrame,failRun);
        await until(()=>{checkChild(stream,'Selkies adapter');return packets.ready},'read-only installed Selkies adapter ready',15000);
        renew=setInterval(()=>packets.write('{"kind":"renew"}\n'),15000);
        metadata.packet_dimensions=packets.dimensions;
      },
      async stop(){clearInterval(renew);packets?.close();stream?.stdin.end();if(stream)await Promise.race([waitChild(stream),pause(5000)]);stream=null;packets?.check()},
      ack(id){if(stream&&Number.isInteger(id)&&id>=0&&id<=65535)packets.write(JSON.stringify({kind:'control',text:`CLIENT_FRAME_ACK ${id}`})+'\n')},
    };
  }catch(e){await close();throw e}
}
