import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

for (const kind of ['clipboard', 'secret-input']) {
  for (const seam of ['exact-read', 'OCR']) {
    test(`MP-11 ${kind} ${seam} failure never logs its retained input`, async () => {
      const source = await readFile(new URL(`./live-computer-${kind}-x11-drill.mjs`, import.meta.url), 'utf8');
      const canary = 'MP11_FAILURE_EVIDENCE_CANARY';
      const failure = new assert.AssertionError({ actual: canary, expected: 'different', operator: seam });
      const reportThrow = source.match(/if \(failure\) throw [^\n]+/)[0];
      assert.throws(() => new Function('failure', reportThrow)(failure), error => {
        assert.equal(String(error.stack).includes(canary), false, 'final rejection is safe');
        return true;
      });
      const logs = [];
      const processState = {};
      const tail = source.slice(source.lastIndexOf('main().catch('));
      new Function('main', 'console', 'process', 'interruptedSignal', tail)(
        () => Promise.reject(failure), { error: text => logs.push(text) }, processState, null,
      );
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(processState.exitCode, 1);
      assert.equal(logs.length, 1);
      assert.equal(logs.join('\n').includes(canary), false, 'stderr is safe even for pre-report failures');
    });
  }
}
