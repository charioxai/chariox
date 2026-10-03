// MP-08/MP-10/MP-11: queued promotion regression, unrelated history and duplicates.
import test from 'node:test';import assert from 'node:assert/strict';import {completedDrillTurn} from './drill-turn-admission.mjs';
const turn=(id,text,life='completed')=>({prompt_id:id,lifecycle:life,entries:[{entry:{kind:'user_prompt',text}}]});
test('queued admission correlates to its promoted fixture turn',()=>{const t=turn('promoted-2','fixture work');assert.equal(completedDrillTurn([t],'pending-1','Queued','fixture work',new Set()),t)});
test('prior identical and unrelated completed turns do not satisfy queued work',()=>{assert.equal(completedDrillTurn([turn('old','fixture work'),turn('unrelated','other')],'pending-1','Queued','fixture work',new Set(['old'])),undefined)});
test('duplicate promoted fixture completions fail',()=>{assert.throws(()=>completedDrillTurn([turn('one','fixture work'),turn('two','fixture work')],'pending-1','Queued','fixture work',new Set()),/duplicate/)});
test('started outcome requires its accepted identity',()=>{assert.equal(completedDrillTurn([turn('other','fixture work')],'started-1','Started','fixture work',new Set()),undefined)});
