// MP-08/MP-10/MP-11: Decision G sampler on this runner's private X display.
import {spawn} from 'node:child_process';import readline from 'node:readline';
export class PhysicalPixels{
 constructor(env){this.env=env;this.next=0;this.pending=new Map();this.active=null}
 async start(){
  this.child=spawn('/usr/bin/python3',[new URL('./pixel-probe.py',import.meta.url).pathname],{env:{...process.env,...this.env},stdio:['pipe','pipe','inherit']});
  this.ready=new Promise((r,j)=>{this.resolveReady=r;this.child.once('error',j)});
  this.lines=readline.createInterface({input:this.child.stdout});this.lines.on('line',s=>{const v=JSON.parse(s);if(v.kind==='ready')this.resolveReady();else{const match=this.pending.get(v.id);if(match&&match.kind===v.kind){clearTimeout(match.timer);this.pending.delete(v.id);match.resolve(v)}}});
  await this.ready;return this;
 }
 send(op,args={},kind=op){const id=args.id??++this.next;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MP-10 native observer deadline'))},7000);this.pending.set(id,{kind,resolve,reject,timer});this.poke(op,{...args,id})})}
 poke(op,args){this.child.stdin.write(JSON.stringify({op,...args})+'\n')}
 async roi(p,css){
  const g=await this.send('geometry',{},'geometry');const {windowId}=await p.context().newCDPSession(p).then(async c=>{const w=await c.send('Browser.getWindowForTarget');await c.detach();return w});
  const layout=await p.evaluate(()=>({dpr:devicePixelRatio,outerW:outerWidth,outerH:outerHeight,innerW:innerWidth,innerH:innerHeight}));
  const chromeY=(layout.outerH-layout.innerH)*layout.dpr;
  const roi=[Math.round(g.bounds.left+css.x*layout.dpr),Math.round(g.bounds.top+chromeY+css.y*layout.dpr),Math.round(css.width*layout.dpr),Math.round(css.height*layout.dpr)];
  if(roi[0]<0||roi[1]<0||roi[0]+roi[2]>g.root.width||roi[1]+roi[3]>g.root.height||roi[2]>512||roi[3]>512)throw Error('MP-10 native ROI outside own display');
  return {roi,geometry:g,layout};
 }
 async arm(p,css,mode='latency',options={}){
  const id=++this.next,g=await this.roi(p,css);const armed=await this.send('arm',{id,roi:g.roi,mode,...options,...(process.env.VIDLAT_FIRST_TARGET_CHANGE==='1'?{first_target_change:true}:{})},'armed');g.baseline_mean=armed.baseline_mean;g.target_already_visible=armed.target_already_visible;this.active={id,mode};
  if(mode==='latency')this.measurement=new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.pending.delete(id);reject(Error('MP-10 missing native input/change'))},6500);this.pending.set(id,{kind:'latency',resolve,reject,timer})});
  return g;
 }
 begin(at){if(this.active)this.poke('begin',{id:this.active.id,at})}
 async result(){try{return await this.measurement}finally{this.active=null}}
 async end(start,end){const v=await this.send('end',{id:this.active.id,start,end},'frames');this.active=null;return v}
 async close(){
  if(!this.child)return;this.child.stdin.end();
  const p=this.child;if(!Number.isSafeInteger(p.pid)||p.pid<=1)throw Error('MP-11 unsafe pixel observer PID');
  if(p.exitCode===null&&p.signalCode===null){await Promise.race([new Promise(r=>p.once('exit',r)),new Promise(r=>setTimeout(r,1000))]);if(p.exitCode===null&&p.signalCode===null)p.kill('SIGTERM')}
  for(const v of this.pending.values()){clearTimeout(v.timer);v.reject(Error('MP-10 observer closed'))}this.pending.clear();this.lines?.close();
 }
}
