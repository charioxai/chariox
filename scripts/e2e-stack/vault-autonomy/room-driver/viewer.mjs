// MP-08 / MP-09 / MP-10 / MP-11: the owner's real web view of the Room (keeps owner focus for browser authority).
// Sign-in uses the real form; the owner password is read from ROOM_OWNER_PASSWORD_FILE and never printed.
// Commands: files in ROOM_DIR/shotcmd: "c-<x>-<y>*" clicks then screenshots, "key-<Key>*" presses a key,
// "type*" types the file content into the prompt bar, any other name only takes a screenshot.
import {mkdir, readdir, readFile, unlink, writeFile} from 'node:fs/promises'

const {ROOM_DIR: dir, ROOM_ORIGIN: origin, ROOM_SESSION: sessionId, ROOM_EVIDENCE: evidence, ROOM_OWNER_EMAIL: email, ROOM_OWNER_PASSWORD_FILE: passwordFile, ROOM_PLAYWRIGHT: playwright, ROOM_CHROME: chrome = '/opt/google/chrome/chrome'} = process.env
if (!dir || !origin || !sessionId || !evidence || !email || !passwordFile || !playwright) throw Error('Set ROOM_DIR ROOM_ORIGIN ROOM_SESSION ROOM_EVIDENCE ROOM_OWNER_EMAIL ROOM_OWNER_PASSWORD_FILE ROOM_PLAYWRIGHT')
const {chromium} = await import(playwright)
await mkdir(`${dir}/shotcmd`, {recursive: true}); await mkdir(evidence, {recursive: true})
const browser = await chromium.launch({headless: true, executablePath: chrome, args: ['--no-sandbox', '--disable-dev-shm-usage']})
const page = await (await browser.newContext({viewport: {width: 1440, height: 900}, deviceScaleFactor: Number(process.env.DPR ?? 1)})).newPage()
await page.goto(`${origin}/auth/login?returnTo=%2Fterminal%3Fui%3Dos`, {waitUntil: 'domcontentloaded'})
await page.getByLabel('Email', {exact: true}).fill(email)
try { await page.getByLabel('Access key', {exact: true}).fill((await readFile(passwordFile, 'utf8')).trim()) } catch { throw Error('private password fill failed') }
await page.getByRole('button', {name: 'Sign in', exact: true}).click()
await page.waitForURL(url => url.pathname === '/terminal', {timeout: 60000})
await page.goto(`${origin}/terminal?ui=os`, {waitUntil: 'domcontentloaded'})
const open = page.locator(`[data-room-id='${sessionId}'] [data-action=open-room]`)
await open.waitFor({timeout: 90000}); await open.click()
await page.locator('[data-prompt-input]').waitFor({timeout: 60000})
await page.screenshot({path: `${evidence}/viewer-room.png`})
await writeFile(`${dir}/viewer.pid`, String(process.pid))
for (;;) {
  for (const name of await readdir(`${dir}/shotcmd`)) {
    const body = await readFile(`${dir}/shotcmd/${name}`, 'utf8'); await unlink(`${dir}/shotcmd/${name}`)
    const click = /^c-(\d+)-(\d+)/.exec(name)
    if (click) { await page.mouse.click(Number(click[1]), Number(click[2])); await page.waitForTimeout(1500) }
    if (name.startsWith('key-')) await page.keyboard.press(name.slice(4).split('.')[0])
    if (name.startsWith('type')) { const field = page.locator('[data-prompt-input]'); await field.fill(body); await field.press('Enter'); await page.waitForTimeout(1500) }
    await page.screenshot({path: `${evidence}/${name.replace(/[^a-z0-9._-]/gi, '')}.png`})
  }
  await page.waitForTimeout(1500)
}
