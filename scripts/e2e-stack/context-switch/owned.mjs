import {assertNotTerminating} from './termination.mjs';
// MP-08 / MP-10 / MP-11: preserve captured generation handles through all cleanup signals.
import {spawnOwned as launch,signalOwnedProcessGroup,signalOwnedProcess,ownedProcessHandles,processSnapshot} from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs';
export {signalOwnedProcessGroup,processSnapshot};
const roots=new Map(),handles=new Map();
export function spawnOwned(command,args,options={}){assertNotTerminating();const child=launch(command,args,{...options,retainLeader:options.detached===true});if(!Number.isInteger(child.pid)||child.pid<=1)throw Error('invalid owned PID');roots.set(child.pid,child);return child}
export function descendants(root){const rows=processSnapshot(),ids=new Set([root]);let changed=true;while(changed){changed=false;for(const r of rows)if(ids.has(r.ppid)&&!ids.has(r.pid)){ids.add(r.pid);changed=true}}const child=roots.get(root);if(child)for(const h of ownedProcessHandles(child))handles.set(h.pid,h);return rows.filter(r=>ids.has(r.pid)&&r.pid>1)}
export function signalOwnedPid(pid,signal){if(!Number.isInteger(pid)||pid<=1)throw Error('invalid owned PID');const handle=handles.get(pid);if(!handle)throw Error('missing captured owned process generation');return signalOwnedProcess(handle,signal)}
