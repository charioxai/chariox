// MD-DISPLAY-02: one lane-owned software RGB encoder; not a product adapter.
import {spawn} from 'node:child_process';
import {createInterface} from 'node:readline';
import {fileURLToPath} from 'node:url';
import {stopGroup} from './runtime.mjs';
export function nativeEncoder(){
 const child=spawn('python3',['-u',fileURLToPath(new URL('./native-encoder.py',import.meta.url))],{detached:true,env:{...process.env,PYTHONPATH:process.env.MD_PY_TOOLS||'/root/.chariox/dev/browser-resume-20260930/agents/display/tools-py'},stdio:['pipe','pipe','ignore']});
 let id=0;const waiting=new Map(),cost=[];
 createInterface({input:child.stdout}).on('line',line=>{const value=JSON.parse(line),p=waiting.get(value.id);if(!p)return;waiting.delete(value.id);value.error?p.reject(Error(value.error)):p.resolve(value)});
 child.on('exit',code=>{for(const p of waiting.values())p.reject(Error(`MD-DISPLAY native encoder exited ${code}`));waiting.clear()});
 const request=value=>new Promise((resolve,reject)=>{const key=++id;waiting.set(key,{resolve,reject});child.stdin.write(JSON.stringify({id:key,...value})+'\n')});
 return {child,cost,reset:()=>request({reset:true}),async encode(png,timestamp){const value=await request({png:png.toString('base64'),timestamp,bitrate:Number(process.env.MD_BITRATE||2000000),crf:Number(process.env.MD_NATIVE_CRF||18),preset:process.env.MD_NATIVE_PRESET||'ultrafast'});cost.push(value.elapsed_ms);return value},async close(){child.stdin.end();await stopGroup(child)}};
}
