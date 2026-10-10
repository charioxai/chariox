import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizeStages } from './drill-stages.mjs';
test('MD-DISPLAY stage correlation excludes bootstrap and retains nested spans', () => {
  const span = (stage, start, end) => ({ stage, started_ms:start, ended_ms:end, duration_ms:end-start });
  const result = summarizeStages({
    probes:[{started_ms:100,drawn_ms:140,presented_ms:150}],
    host_timings:[span('protected_capture',105,125),span('png_decode',125,130),span('bootstrap',0,99)],
    kernel_timings:[span('event_queue_credit',130,131)],
    client_timings:[span('event_received',134,134),span('client_decode',134,139)],
  });
  assert.equal(result.stages.bootstrap, undefined);
  assert.equal(result.stages.protected_capture.p50_ms, 20);
  assert.equal(result.stages.queued_event_to_viewer.p50_ms, 3);
  assert.equal(result.stages.input_to_raf.p50_ms, 50);
  assert.equal(result.stages.draw_to_raf.p50_ms, 10);
});
