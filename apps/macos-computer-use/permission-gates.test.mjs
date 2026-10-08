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

test('AX mutation and observation keep permission, ownership and hit-test fences', () => {
  const ax = native.slice(native.indexOf('func performAX('), native.indexOf('func perform('));
  assert.match(ax, /checkPermission\(operation\)[\s\S]*fence\(request, element: element, typing: false\)[\s\S]*AXUIElementSetAttributeValue/);
  assert.match(ax, /AXUIElementSetAttributeValue[\s\S]*AXUIElementPerformAction/);
  assert.match(ax, /Task.sleep[\s\S]*checkPermission\(operation\)[\s\S]*fence\(request, element: element, typing: false\)[\s\S]*scrollValue/);
  assert.match(ax, /bind\(scroller, to: element, pid: request.pid\)/);
  assert.match(native, /bind\(element, to: window, pid: request.pid\)[\s\S]*bounds\(window\).contains\(try bounds\(element\)\)/);
  assert.match(native, /AXUIElementCopyElementAtPosition[\s\S]*guard matched/);
  assert.doesNotMatch(ax, /postToPid|post\(tap:|kAXFocusedAttribute|kAXRaiseAction/);
});

test('input has one role-selected path and no global posting or cursor warp', () => {
  assert.match(native, /if path == .axPress \|\| path == .axScrollValue \{\s*return try await performAX/);
  assert.doesNotMatch(native, /post\(tap:|CGWarpMouseCursorPosition|activate\(ignoringOtherApps/);
});
