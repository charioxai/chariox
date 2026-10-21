// MP-08 / MP-10 / MP-11: one termination path owns all detached drill resources.
const cleanup=new Set();let terminating=false;
export function assertNotTerminating(){if(terminating)throw Error('Context-switch drill is terminating');}
export function registerTerminationCleanup(close){cleanup.add(close);return()=>cleanup.delete(close);}
export async function acquireTerminationResource(start,close){
 assertNotTerminating();const pending=start();registerTerminationCleanup(async()=>close(await pending));
 const resource=await pending;assertNotTerminating();return resource;
}
async function terminate(signal){
 if(terminating)return;terminating=true;const failures=[];
 while(cleanup.size){const close=[...cleanup].at(-1);cleanup.delete(close);try{await close();}catch(error){failures.push(error?.name??'Error');}}
 console.error('MP-08 / MP-10 / MP-11 termination cleanup '+signal+': '+(failures.length?'FAIL '+failures.join(','):'PASS'));
 process.exit(failures.length?1:signal==='SIGINT'?130:143);
}
process.on('SIGINT',()=>void terminate('SIGINT'));
process.on('SIGTERM',()=>void terminate('SIGTERM'));

process.on('uncaughtException',error=>{if(!terminating)throw error;});
