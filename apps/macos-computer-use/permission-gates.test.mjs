// Structural regression checks run without native permission, capture or input calls.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const native = readFileSync(new URL('./Native.swift', import.meta.url), 'utf8');
const policy = readFileSync(new URL('./Policy.swift', import.meta.url), 'utf8');

test('run owns the pre-dispatch permission gate', () => {
  const dispatch = policy.slice(policy.indexOf('for operation in request.operations {', policy.indexOf('var result:')));
  assert.match(dispatch, /validateTarget\(request\)[\s\S]*checkPermission\(operation\)[\s\S]*source.perform\(operation/);
  const target = native.slice(native.indexOf('func target('), native.indexOf('func bounds('));
  const perform = native.slice(native.indexOf('func perform('));
  const beforeCaptureAwait = perform.slice(0, perform.indexOf('try await'));
  assert.doesNotMatch(target, /AXIsProcessTrustedWithOptions/);
  assert.doesNotMatch(beforeCaptureAwait, /checkPermission|CGPreflightScreenCaptureAccess/);
});

test('capture completion and event posting retain permission fences', () => {
  assert.match(native, /await SCScreenshotManager.captureImage[\s\S]*guard CGPreflightScreenCaptureAccess\(\)/);
  assert.match(native, /guard CGPreflightPostEventAccess\(\) else[\s\S]*for event in events/);
});
