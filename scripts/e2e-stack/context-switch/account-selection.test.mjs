// MP-08 / MP-10 / MP-11: the real CLI accepts registered account labels.
import test from 'node:test';
import assert from 'node:assert/strict';
import {accountSelectionCommand} from './account-selection.mjs';
test('MP-08 / MP-10 / MP-11 sends the exact display label while retaining the target profile ID',()=>{
 const input={agentId:'agent-2',profileId:'immutable-secondary-id',profiles:[{profile_id:'primary-id',label:'Primary'},{profile_id:'immutable-secondary-id',label:'Independent work account'}]};
 assert.equal(accountSelectionCommand(input),'agent account agent-2 Independent work account');
 assert.equal(input.profileId,'immutable-secondary-id');
});
test('MP-08 / MP-10 / MP-11 rejects an unregistered profile before sending a CLI command',()=>{
 assert.throws(()=>accountSelectionCommand({agentId:'agent-2',profileId:'missing',profiles:[]}),/registered account label/);
});
test('MP-08 / MP-10 / MP-11 rejects a label that would enter another terminal command',()=>{
 for(const label of ['', 'Work\n/stop','Work\r/stop'])assert.throws(()=>accountSelectionCommand({agentId:'agent-2',profileId:'secondary',profiles:[{profile_id:'secondary',label}]}),/registered account label/);
});
