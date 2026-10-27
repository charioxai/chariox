// MP-08/MP-10: kernel human actors use user IDs; prefixes are not authority.
import test from 'node:test';
import assert from 'node:assert/strict';
import {isHumanRoomReload} from './wp07p-drill-e-realism.mjs';
test('MP-08/MP-10 resolves human attribution from the authoritative actor kind',()=>{
 const environment={actors:[{actor_id:'user:local',kind:'human'},{actor_id:'human:agent',kind:'agent'}]};
 const action={sequence:12,kind:'browser_history_reload',state:'completed'};
 assert.equal(isHumanRoomReload({...action,actor_id:'user:local'},environment,9),true);
 assert.equal(isHumanRoomReload({...action,actor_id:'human:agent'},environment,9),false);
});
