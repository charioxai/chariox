import test from 'node:test';
import assert from 'node:assert/strict';
import { browserMajor, headlessIdentityArguments, headlessUserAgent, readBrowserMajor } from './kernel-browser-identity.mjs';
import { launchArguments } from './kernel-browser-process.mjs';

test('MP-08/10/11 the browser major comes from the version line of every build the kernel launches', () => {
  assert.equal(browserMajor('Google Chrome for Testing 155.0.8059.79 \n'), 155);
  assert.equal(browserMajor('Chromium 155.0.8059.79 built on Debian'), 155);
  assert.equal(browserMajor('Google Chrome 155.0.7339.2'), 155);
  assert.throws(() => browserMajor('no version here'), /cannot read the browser version/);
});

test('MP-08/10/11 a failed version read says what to do and a missing binary is not mistaken for a version', async () => {
  await assert.rejects(readBrowserMajor('/nope', {}, async () => { throw new Error('ENOENT'); }), /cannot run the browser to read its version/);
  await assert.rejects(readBrowserMajor('/x', {}, async () => ({ stdout: 'garbage' })), /cannot read the browser version/);
  assert.equal(await readBrowserMajor('/x', {}, async () => ({ stdout: 'Chromium 155.0.1.2' })), 155);
});

test('MP-08/10/11 headless Chrome is given the User-Agent and pointer of the headed Chrome on the same platform', () => {
  const linux = headlessIdentityArguments('linux', 155);
  assert.equal(linux[0], '--user-agent=Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36');
  const mac = headlessIdentityArguments('darwin', 155);
  assert.equal(mac[0], '--user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36');
  for (const args of [linux, mac]) { assert.ok(!args.join(' ').includes('Headless')); assert.ok(args.some(a => a.startsWith('--blink-settings=') && a.includes('primaryPointerType=4') && a.includes('primaryHoverType=2'))); }
  assert.throws(() => headlessIdentityArguments('win32', 155), /no headed identity/);
});

test('MP-08/10/11 the identity arguments reach the launch line before the start page', () => {
  const args = launchArguments('/private', true, false, 1, [], headlessIdentityArguments('linux', 155));
  assert.ok(args.includes('--headless=new'));
  assert.ok(args.findIndex(a => a.startsWith('--user-agent=')) < args.indexOf('about:blank'));
  assert.ok(!launchArguments('/private', false, true).some(a => a.startsWith('--user-agent=')), 'a headed browser keeps Chrome\'s own identity');
});

// The regression suite asserts the User-Agent for the platform it runs on; this proves both tokens are chosen by platform
// (the macOS branch cannot run on the Linux builder, only this unit check covers it there).
test('MP-08/10/11 the expected headless User-Agent follows the platform the launcher uses', () => {
  const ua = token => `Mozilla/5.0 (${token}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36`;
  assert.equal(headlessUserAgent('linux', 155), ua('X11; Linux x86_64'));
  assert.equal(headlessUserAgent('darwin', 155), ua('Macintosh; Intel Mac OS X 10_15_7'));
  assert.notEqual(headlessUserAgent('linux', 155), headlessUserAgent('darwin', 155));
  for (const platform of ['linux', 'darwin']) assert.equal(headlessIdentityArguments(platform, 155)[0], `--user-agent=${headlessUserAgent(platform, 155)}`, 'the launch argument and the expectation come from one place');
  assert.throws(() => headlessUserAgent('win32', 155), /no headed identity/);
});
