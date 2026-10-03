// MP-08/MP-10: owned failure evidence, independent of coordinator/gate mutation.
export async function captureRoomRepetitionFailure({docker,write,sliceContainer,webContainer,privateRelayProbe}) {
 for(const container of [sliceContainer,webContainer].filter(Boolean)) {
  const cpuStat=await docker(['exec',container,'cat','/sys/fs/cgroup/cpu.stat']).catch(()=>null)
  await write(`failure-cpu-${container}`,{container,cpuStat})
 }
 if(!privateRelayProbe||!sliceContainer)return
 const codes=await docker(['exec','-u','root',sliceContainer,'cat','/tmp/loops-private-relay-codes.jsonl']).catch(()=>null)
 const records=[];let invalidLines=0
 for(const line of codes?.split('\n').filter(Boolean)??[]){try{records.push(JSON.parse(line))}catch{invalidLines++}}
 await write('private-relay-close-codes',{scope:'own slice loopback metadata only; no buffering change',records,invalidLines})
}
