// MP-08/MP-10/MP-11: preserve each pipe's byte order for diagnostic parsing.
import {writeFile} from 'node:fs/promises';
import path from 'node:path';

export class KernelLogCapture {
 #combined=[];
 #streams={stdout:[],stderr:[]};
 record(stream,bytes) {
  if(!Object.hasOwn(this.#streams,stream))throw Error('MP-10: unknown kernel log stream');
  this.#combined.push(bytes);this.#streams[stream].push(bytes);
 }
 timings() {
  return Buffer.concat(this.#streams.stderr).toString().split('\n')
   .filter(line=>line.startsWith('MD-DISPLAY-TIMING '))
   .map(line=>JSON.parse(line.slice('MD-DISPLAY-TIMING '.length)));
 }
 async writeTo(directory) {
  await writeFile(path.join(directory,'kernel.log'),Buffer.concat(this.#combined));
  for(const stream of ['stdout','stderr'])
   await writeFile(path.join(directory,`kernel.${stream}.log`),Buffer.concat(this.#streams[stream]));
 }
}
