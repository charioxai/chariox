// own CLI cloud login (device flow, approved in headless Chromium) for the client-boundary forging cases
import './client-env.mjs'
import assert from 'node:assert/strict'
const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const { CloudClient } = await import(`${repo}/apps/cli/dist/cloud-client.js`)
const { saveRelayCloudProfile } = await import(`${repo}/apps/cli/dist/preferences.js`)
const { chromium } = await import(process.env.ADMISSION_DRILL_PLAYWRIGHT)
const origin = process.argv[2], client = new CloudClient()
let browser
try {
  assert.ok(process.env.CHARIOX_HOME.startsWith(process.env.ADMISSION_DRILL_CLI_HOME))
  browser = await chromium.launch({ headless: true, executablePath: '/opt/google/chrome/chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] })
  const page = await browser.newPage()
  const profile = await client.login(origin, async verification => {
    assert.equal(new URL(verification.verificationUrl).origin, origin)
    await page.goto(verification.verificationUrl)
    const approve = page.getByRole('button', { name: /approve|allow|connect/i })
    if (await approve.count() === 1) await approve.click()
  })
  await saveRelayCloudProfile(profile)
  console.log('logged in', profile.accountId?.slice(0, 6))
} catch (e) { console.error(String(e.message).slice(0, 300)); process.exitCode = 1 }
finally { client.stop(); await browser?.close() }
