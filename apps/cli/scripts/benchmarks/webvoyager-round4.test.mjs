// MP-08/MP-10/MP-11: vision permission is one protected observation, never Computer mutation.
import test from 'node:test'
import assert from 'node:assert/strict'
import { webVoyagerPrompt, webVoyagerToolAllowed } from './webvoyager-task.mjs'

test('MP-08/MP-10 vision prompt requests protected image blocks and preserves restrictions', () => {
  const prompt = webVoyagerPrompt({ ques: 'Read a plot.', web: 'https://example.org' }, 'recovery-vision-v1')
  assert.match(prompt, /vision-allowed/)
  assert.match(prompt, /slice_screenshot.*return_image_base64=true/)
  assert.match(prompt, /Do not accept optional tracking/)
  assert.match(prompt, /Refresh observed field IDs/)
  assert.match(prompt, /no logins, sign-ups, purchases/)
  assert.match(prompt, /No shell, files, scripts, direct HTTP/)
  assert.match(prompt, /15 mutating Browser actions, 80 Browser tool calls and 600 seconds/)
  assert(!prompt.includes('For image-only results, look for an observed Plain Text'))
})
test('MP-11 allowlist permits only the existing screenshot observation in vision mode', () => {
  for (const variant of ['frozen', 'recovery-v1', 'recovery-vision-v1']) {
    assert(webVoyagerToolAllowed('slice_browser_text', variant))
    assert.equal(webVoyagerToolAllowed('slice_screenshot', variant), variant === 'recovery-vision-v1')
    for (const tool of ['slice_mouse', 'slice_keyboard', 'slice_ocr', 'shell', 'read_file', 'slice_screen_status']) {
      assert.equal(webVoyagerToolAllowed(tool, variant), false)
    }
  }
})
