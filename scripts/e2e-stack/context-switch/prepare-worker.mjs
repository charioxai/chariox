// MP-08 / MP-10 / MP-11: prepare only public exact-head guest inputs.
import {mkdir,writeFile,copyFile,rename} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {lane,sourceTree,verifyBuild,harness} from './paths.mjs';
const build=await verifyBuild('921',sourceTree);
const out=lane+'/public';await mkdir(out,{recursive:true,mode:0o700});
execFileSync('git',['-C',sourceTree,'archive','--format=tar','--output='+out+'/source.tar',build.head]);
execFileSync('gzip',['-f',out+'/source.tar']);await rename(out+'/source.tar.gz',out+'/source.tgz');
await writeFile(out+'/source-head',build.head+'\n');
execFileSync('tar',['-czf',out+'/client.tgz','-C',lane,'client']);
execFileSync('tar',['-czf',out+'/tooling.tgz','-C',harness,'.']);
for(const name of ['kernel-921','ready-921'])await copyFile(lane+'/bin/'+name,out+'/'+name);
for(const [name,path] of [['node',process.env.CTXVAL_NODE_BIN??process.execPath],['claude',process.env.CHARIOX_CLAUDE_BIN]]){
 if(!path)throw Error('CHARIOX_CLAUDE_BIN must name the installed official CLI');await copyFile(path,out+'/'+name);
}
await writeFile(out+'/bridge-service.mjs',`import net from 'node:net';import {spawn} from 'node:child_process';
net.createServer(socket=>{const child=spawn(process.execPath,['/opt/drill/harness/worker-rpc.mjs'],{env:{...process.env,CTXVAL_LANE:'/opt/drill',CTXVAL_STATE_ROOT:'/root/.chariox/dev/context-switch-worker',CTXVAL_SIGNAL_MODULE:'/opt/drill/source/apps/kernel/slice-linux-docker/owned-process-signals.mjs'},stdio:['pipe','pipe','inherit']});socket.pipe(child.stdin);child.stdout.pipe(socket);socket.on('error',()=>socket.destroy());socket.on('close',()=>child.stdin.end());child.on('exit',()=>socket.end());}).listen(34567,'0.0.0.0');\n`,{mode:0o600});
console.log('MP-08 / MP-10 / MP-11: prepared public guest inputs for '+build.head);
