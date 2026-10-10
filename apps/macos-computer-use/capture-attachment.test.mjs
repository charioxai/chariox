import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import test from 'node:test';
const source = file => readFileSync(new URL(file, import.meta.url), 'utf8');
test('paired screenshot uses protected in-memory capture and returns the existing screenshot shape', () => {
  assert.match(source('./PairedSession.swift'), /capture\.screenshot/);
  assert.ok(existsSync(new URL('./PairedCapture.swift', import.meta.url)));
  const capture = source('./PairedCapture.swift');
  assert.match(capture, /desktopIndependentWindow/);
  assert.match(capture, /data_base64/);
  assert.doesNotMatch(capture, /\.write\(|requestAccess|CGRequest|AXValueAttribute/);
});
test('protection changes flush damage bases before any next publication', () => {
  assert.match(source('./PairedSession.swift'), /capture\.protect/);
  assert.match(source('./CaptureDamage.swift'), /func invalidate/);
});
