// MP-08/MP-10/MP-11: preserve each pipe's byte order for diagnostic parsing.
import {writeFile} from 'node:fs/promises';
import path from 'node:path';

export class KernelLogCapture {
 #combined=[];
 #streams={stdout:[],stderr:[]};
 diagnosticErrors=[];
 record(stream,bytes) {
  if(!Object.hasOwn(this.#streams,stream))throw Error('MP-10: unknown kernel log stream');
  this.#combined.push(bytes);this.#streams[stream].push(bytes);
 }
 timings() {
  const records=[];this.diagnosticErrors=[];
  for(const [index,line] of Buffer.concat(this.#streams.stderr).toString().split('\n').entries()) {
   if(!line.startsWith('MD-DISPLAY-TIMING '))continue;
   try{records.push(JSON.parse(line.slice('MD-DISPLAY-TIMING '.length)))}
   catch{this.diagnosticErrors.push({stream:'stderr',line:index+1,error:'invalid timing JSON'})}
  }
  return records;
 }
 async writeTo(directory) {
  await writeFile(path.join(directory,'kernel.log'),Buffer.concat(this.#combined));
  for(const stream of ['stdout','stderr'])
   await writeFile(path.join(directory,`kernel.${stream}.log`),Buffer.concat(this.#streams[stream]));
 }
}

// MP-08/MP-10/MP-11: polling a closed gate is not another closure. Older
// builds/disabled diagnostics have unknown closure counts, never a false zero.
export function summarizePushGate(records) {
 const samples=stage=>records.filter(row=>row.stage===stage&&Number.isFinite(row.duration_ms)).map(row=>row.duration_ms);
 const observed=records.some(row=>['push_gate_started','push_gate_closed','push_gate_reopened'].includes(row.stage));
 return {observed,closures:observed?samples('push_gate_closed').length:null,
  reopened:observed?samples('push_gate_reopened').length:null,
  closed_at_age_ms:samples('push_gate_closed'),closed_duration_ms:samples('push_gate_reopened'),
  oldest_unacked_poll_age_ms:samples('push_oldest_unacked'),frame_ack_age_ms:samples('push_frame_ack'),ack_transport_rtt_ms:samples('push_ack_rtt')};
}
