import assert from 'node:assert/strict';
import test from 'node:test';
import {deliverBrowserImport, expectedDeliveryEnvelope} from './delivery-adapter.mjs';

test('missing production runtime delivery fails closed and never simulates success', async () => {
  const secretBatch = {cookies:[{name:'session',value:'private-marker'}],summary:{count:1,domains:['example.com']}};
  await assert.rejects(deliverBrowserImport({batch:secretBatch}),error => {
    assert.equal(error.code,'browser_import_delivery_unavailable');
    assert.equal(error.message,'browser_import_delivery_unavailable');
    assert.equal(JSON.stringify(error).includes('private-marker'),false);
    return true;
  });
});

test('documented delivery envelope is exact metadata plus an encrypted payload placeholder', () => {
  assert.deepEqual(Object.keys(expectedDeliveryEnvelope),[
    'command','request_id','session_id','attachment_id','environment_id','runtime_generation',
    'tab_id','document_revision','source_store_id','domains','partition_sites','overwrite',
    'encrypted_cookie_batch',
  ]);
  assert.deepEqual(Object.keys(expectedDeliveryEnvelope.encrypted_cookie_batch),[
    'sender_public_key','nonce','ciphertext',
  ]);
});
