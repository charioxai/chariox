import assert from 'node:assert/strict';
import test from 'node:test';
import {PermissionGrantCoordinator,snapshotPermissionState} from './permission-coordinator.mjs';

const permission = {permissions:['cookies'],origins:['*://example.com/*','*://login.example.com/*']};

test('snapshot records cookies and every exact host independently', async () => {
  const queries = [];
  const chrome = {permissions:{contains:async value => {
    queries.push(value);
    return value.permissions?.[0] === 'cookies' || value.origins?.[0] === '*://example.com/*';
  }}};
  assert.deepEqual(await snapshotPermissionState(chrome,permission),{
    cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':false}});
  assert.deepEqual(queries,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('preexisting grants are never revoked and operation-acquired grants are', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  coordinator.reserve('one',permission,{cookies:true,
    origins:{'*://example.com/*':true,'*://login.example.com/*':false}});
  coordinator.activate('one');
  await coordinator.release('one');
  assert.deepEqual(removed,[{origins:['*://login.example.com/*']}]);
});

test('concurrent pages retain shared transient grants until the final operation releases', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  const none = {cookies:false,origins:{'*://example.com/*':false,'*://login.example.com/*':false}};
  const observed = {cookies:true,origins:{'*://example.com/*':true,'*://login.example.com/*':true}};
  coordinator.reserve('first',permission,none);
  coordinator.activate('first');
  coordinator.reserve('second',permission,observed);
  coordinator.activate('second');
  await coordinator.release('first');
  assert.deepEqual(removed,[]);
  await coordinator.release('second');
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});

test('denial before activation releases reservation without revoking anything', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  coordinator.reserve('denied',permission,{cookies:false,origins:{'*://example.com/*':false,
    '*://login.example.com/*':false}});
  await coordinator.release('denied');
  assert.deepEqual(removed,[]);
});

test('permission-added observation cleans a grant when its page closes before activation acknowledgement', async () => {
  const removed = [];
  const coordinator = new PermissionGrantCoordinator(async value => {removed.push(value); return true;});
  coordinator.reserve('closing',permission,{cookies:false,origins:{'*://example.com/*':false,
    '*://login.example.com/*':false}});
  coordinator.observeAdded(permission);
  await coordinator.release('closing');
  assert.deepEqual(removed,[{permissions:['cookies']},{origins:['*://example.com/*']},
    {origins:['*://login.example.com/*']}]);
});
