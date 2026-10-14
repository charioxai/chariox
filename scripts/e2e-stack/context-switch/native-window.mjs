// MP-08 / MP-10 / MP-11: official /context through the actual native Chariox client.
import {writeFile} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {spawnOwned,signalOwnedProcessGroup} from './owned.mjs';
import {lane,harness} from './paths.mjs';
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const plain=text=>text.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g,'').replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g,'');
export async function nativeWindow(run,model){
 const alias='context-window-'+Date.now(),args=[lane+'/bin/chariox','claude','--kernel-url',run.url,'--workspace',run.tree,'--worktree',run.tree,'--alias',alias,'--model',model,'--permissions','required'];
 const quote=v=>"'"+v.replaceAll("'","'\\''")+"'";
 const child=spawnOwned('script',['-qefc','stty cols 160 rows 48; exec '+args.map(quote).join(' '),'/dev/null'],{cwd:run.tree,env:{...run.env,TERM:'xterm-256color'},stdio:['pipe','pipe','pipe'],detached:true});
 let output='',trusted=false;
 child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);
 try{
  const end=Date.now()+120000;let ready=false;
  while(Date.now()<end){const text=plain(output);if(child.exitCode!==null||child.signalCode!==null)throw Error('Native context client exited before readiness');if(/trust (?:the files|this folder)|Yes, I trust/i.test(text)&&!trusted){child.stdin.write('\r');trusted=true;}if(/Claude Code/.test(text)&&/[❯>]/.test(text)){ready=true;break;}await sleep(250);}
  if(!ready)throw Error('Native /context composer readiness deadline');
  await sleep(2000);const start=output.length;child.stdin.write('/context\r');
  let tokens;const deadline=Date.now()+30000;
  while(Date.now()<deadline){const text=plain(output.slice(start));for(const m of text.matchAll(/(?:[\d.,]+\s*[kKmM]?\s*\/\s*)([\d.,]+)\s*([kKmM]?)\s*tokens/g)){tokens=Number(m[1].replaceAll(',',''))*({k:1000,m:1000000}[m[2].toLowerCase()]??1);}if(tokens)break;await sleep(250);}
  // The run recorder rejects private values before any evidence is written.
  await run.record(alias,{model,windowTokens:tokens??null,officialCommand:'/context',client:'built chariox claude',ansi:output});
  const file=run.out+'/'+alias+'.ansi';await writeFile(file,output,{mode:0o600});execFileSync('python3',[harness+'/render-pty.py',file],{stdio:'ignore'});
  if(!tokens)throw Error('Official /context did not expose a parseable window; inspect native screenshot');
  return {model,windowTokens:tokens,evidence:alias};
 }finally{
  // These are UI exit keys, never an active provider cancellation drill.
  if(child.exitCode===null&&child.signalCode===null){child.stdin.write('\x1b');await sleep(250);child.stdin.write('/exit\r');for(let n=0;n<50&&child.exitCode===null&&child.signalCode===null;n++)await sleep(100);if(child.exitCode===null&&child.signalCode===null)signalOwnedProcessGroup(child,'SIGTERM');}
 }
}
