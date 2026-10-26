// MP-08 / MP-10 / MP-11: real built TUI, documented automation, owned signals.
import net from 'node:net'
import { writeFile } from 'node:fs/promises'
import { spawnOwned, signalOwnedProcessGroup } from './owned.mjs'
const sleep = ms => new Promise(r => setTimeout(r,ms))
export async function openTui({tree,env,url,profile,session,root,provider="claude-p",model="sonnet",binary,clientId,connectionArgs,prefix=[]}) {
 if(!binary)throw Error('Explicit built TUI required');
 const socketPath=`${root}/tui.sock`
 const args=[...prefix,binary,...(connectionArgs??['--kernel-url',url]),'--automation-socket',socketPath,
  ...(session?['--session',session]:['--create-session']), '--workspace',tree,'--worktree',tree,
  '--provider',provider,'--account-profile',profile,'--model',model,'--client-id',clientId??'ctxval-r5']
 const quote = value => "'"+value.replaceAll("'","'\\''")+"'"
 const child=spawnOwned('script',['-qefc','stty cols 160 rows 48; exec '+args.map(quote).join(' '),'/dev/null'],{
  cwd:tree,env:{...env,TERM:'xterm-256color',COLUMNS:'160',LINES:'48',NODE_OPTIONS:'--max-old-space-size=4096'},
  stdio:['pipe','pipe','pipe'],detached:true,retainLeader:true})
 let output=''; child.stdout.on('data',chunk=>output+=chunk);child.stderr.on('data',chunk=>output+=chunk)
 let socket
 for(let i=0;i<120;i++) {
  if(child.exitCode!==null)throw new Error(`TUI exited with code ${child.exitCode}: ${output.slice(-2500)}`)
  try {socket=await new Promise((resolve,reject)=>{const s=net.createConnection(socketPath);s.once('connect',()=>resolve(s));s.once('error',reject)});break}catch{await sleep(500)}
 }
 if(!socket)throw new Error('TUI automation socket timeout')
 let next=0,buffer='';const pending=new Map();socket.setEncoding('utf8')
 socket.on('data',chunk=>{buffer+=chunk;while(buffer.includes('\n')){
  const at=buffer.indexOf('\n'),line=buffer.slice(0,at);buffer=buffer.slice(at+1);if(!line.trim())continue
  const response=JSON.parse(line),p=pending.get(response.id);if(!p)continue
  pending.delete(response.id);clearTimeout(p.timer);response.ok?p.resolve(response.data):p.reject(new Error(response.error))
 }})
 socket.on('close',()=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(new Error('TUI automation socket closed'))}pending.clear()})
 function send(action,fields={}){if(socket.destroyed)return Promise.reject(new Error('TUI automation socket closed'));const id=++next;return new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{pending.delete(id);reject(new Error(`${action} timeout`))},action==='exit'?5000:360000);timer.unref()
  pending.set(id,{resolve,reject,timer});socket.write(JSON.stringify({id,action,...fields})+'\n')
 })}
 let closed=false;return {child,send,type:async(text)=>{child.stdin.write(text);await sleep(300)},output:()=>output,async close(){
  if(closed)return;closed=true;try {if(!socket.destroyed&&child.exitCode===null&&child.signalCode===null)await send('exit')}catch{}
  socket.destroy();for(let i=0;i<40&&child.exitCode===null;i++)await sleep(100)
  if(child.exitCode===null)signalOwnedProcessGroup(child,'SIGTERM')
  await writeFile(`${root}/tui.ansi`,output,{mode:0o600})
 }}
}
export function publicSnapshot(s){return {
 session:s.session,statusLine:s.statusLine,daemonDisconnected:s.daemonDisconnected,
 interactions:(s.interactions??[]).map(({id,title,choices,customChoice,kind,agentId})=>({id,title,choices,customChoice,kind,agentId})),
 agentPanes:s.agentPanes,transcript:s.transcript,shell:s.shell,footer:s.footer
}}
