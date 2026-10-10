// Structural regression checks run without native permission, capture or input calls.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const native = readFileSync(new URL('./Native.swift', import.meta.url), 'utf8');
const policy = readFileSync(new URL('./Policy.swift', import.meta.url), 'utf8');
const pointer = readFileSync(new URL('./PointerInput.swift', import.meta.url), 'utf8');
const textClick = readFileSync(new URL('./TextClick.swift', import.meta.url), 'utf8');

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
  assert.match(native, /guard CGPreflightPostEventAccess\(\) else[\s\S]*dispatchInputEvents\(events/);
  assert.match(pointer, /for event in events \{\s*try fence\(event\)[\s\S]*post\(event\)/);
  assert.match(pointer, /try fence\(event\)\s*guard releaseAllowed\(\) else \{ throw Refusal.permission \}/);
  assert.match(native, /releaseAllowed: \{ CGPreflightPostEventAccess\(\) \}/);
});

test('AX mutation and observation keep permission, ownership and hit-test fences', () => {
  const ax = native.slice(native.indexOf('func performAX('), native.indexOf('func perform('));
  assert.match(ax, /checkPermission\(operation\)[\s\S]*fence\(request, element: element, typing: false\)[\s\S]*AXUIElementSetAttributeValue/);
  assert.match(ax, /AXUIElementSetAttributeValue[\s\S]*AXUIElementPerformAction/);
  assert.match(ax, /Task.sleep[\s\S]*checkPermission\(operation\)[\s\S]*fence\(request, element: element, typing: false\)[\s\S]*scrollValue/);
  assert.match(ax, /bind\(scroller, to: element, pid: request.pid\)/);
  assert.match(native, /bind\(element, to: window, pid: request.pid\)[\s\S]*geometry\(request, element: element\)[\s\S]*geometry.checkedLocation\(geometry.location, current: geometry\)/);
  assert.match(native, /AXUIElementCopyElementAtPosition[\s\S]*guard matched/);
  assert.doesNotMatch(ax, /postToPid|post\(tap:|kAXFocusedAttribute|kAXRaiseAction/);
});

test('posting fences retain click geometry and hit-test the actual event location', () => {
  assert.match(native, /clickGeometry = geometry[\s\S]*hidClickEvents\(window: request.window, location: geometry.location,\s*windowBounds: geometry.windowBounds/);
  assert.match(native, /dispatchInputEvents\(events, fence: \{ event in[\s\S]*clickGeometry: clickGeometry, eventLocation: event.location/);
  assert.match(native, /location = try clickGeometry.checkedLocation\(eventLocation, current: geometry\)[\s\S]*AXUIElementCopyElementAtPosition\(app, Float\(location.x\), Float\(location.y\)/);
  assert.match(native, /guard matched else[\s\S]*clickGeometry.checkedLocation\(location, current:/);
  assert.match(pointer, /guard current == self, eventLocation == location/);
});

test('owned mouse release validates original process and window without stale geometry', () => {
  const release = native.slice(native.indexOf('func mouseReleaseFence('), native.indexOf('func fixtureCounter('));
  assert.match(release, /event.type == .leftMouseUp, !application.isTerminated/);
  assert.match(release, /application.processIdentifier == request.pid[\s\S]*current.launchDate == launchDate/);
  assert.match(release, /mouseEventWindowUnderMousePointer\) == Int64\(request.window\)/);
  assert.match(release, /mouseEventWindowUnderMousePointerThatCanHandleThisEvent\) == Int64\(request.window\)/);
  assert.match(release, /validateTarget\(request\)[\s\S]*selectedWindow\(request, app: app\)[\s\S]*CFEqual\(selected, window\)/);
  assert.match(release, /bind\(window, to: selected, pid: request.pid\)[\s\S]*focusedElement\(app: app, window: selected\)[\s\S]*checkPermission\(.click\)/);
  assert.doesNotMatch(release, /checkedLocation|clickGeometry|AXUIElementCopyElementAtPosition/);
  assert.match(native, /let admittedLaunchDate = admittedApp.launchDate[\s\S]*let admittedWindow = try selectedWindow/);
  assert.match(native, /releaseFence: \{ event in[\s\S]*mouseReleaseFence\(request, application: admittedApp, launchDate: admittedLaunchDate,\s*window: admittedWindow, event: event\)/);
  assert.match(pointer, /guard let release else \{ throw error \}[\s\S]*try releaseFence\(release\)[\s\S]*guard releaseAllowed\(\)[\s\S]*catch \{ throw Refusal.ownedInput \}/);
  assert.match(policy, /unresolved owned input; owner reset required/);
  const helper = readFileSync(new URL('./Helper.swift', import.meta.url), 'utf8');
  assert.match(helper, /fputs\("\\\(refusalMessage\(error\)\)\\n", stderr\)/);
});

test('HID posting is confined to unresolved text clicks; text typing stays per-PID', () => {
  assert.match(native, /if path == .axPress \|\| path == .axScrollValue \{\s*return try await performAX/);
  assert.match(native, /if let receipt = try await performTextClick[\s\S]*return receipt[\s\S]*hidClickEvents/);
  assert.match(native, /if typing \{ \$0.postToPid\(request.pid\) \} else \{ \$0.post\(tap: .cghidEventTap\) \}/);
  assert.doesNotMatch(native, /CGWarpMouseCursorPosition|activate\(ignoringOtherApps/);
});

test('AX caret mutation and readback keep the saved-point fences and never post', () => {
  assert.match(textClick, /checkPermission\(.click\)[\s\S]*clickGeometry: geometry, eventLocation: geometry.location/);
  assert.match(textClick, /guard case .selection\(let index\) = resolution else \{ return nil \}/);
  assert.match(textClick, /guard try resolved\(\) == resolution else[\s\S]*try checked\(\)[\s\S]*AXUIElementSetAttributeValue\(element, kAXSelectedTextRangeAttribute/);
  assert.match(textClick, /CFRange\(location: index, length: 0\)/);
  assert.match(textClick, /Task.sleep[\s\S]*try checked\(\)[\s\S]*rangeValue\(attribute\(element, kAXSelectedTextRangeAttribute\)\)/);
  assert.match(textClick, /selected.location == index && selected.length == 0/);
  assert.doesNotMatch(textClick, /postToPid|post\(tap:|kAXValueAttribute|kAXSelectedTextAttribute|kAXStringForRange/);
});
