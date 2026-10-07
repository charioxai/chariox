// MP-08/MP-10: operator drill reserves; product runtime has no such floor.
import {totalmem} from 'node:os';
export function memoryFloorGiB(explicit,total=totalmem()){
 const value=explicit===undefined||explicit===''?Math.max(.5,Math.min(2,total/1024**3*.15)):Number(explicit);
 if(!Number.isFinite(value)||value<=0)throw Error('MP-10: invalid resource reserve');return value;
}
