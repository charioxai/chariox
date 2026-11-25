// The scripted person for D3 (HARNESS.md section 4): a Playwright client at DPR1 / DPR2 / phone acting as the owner.
// Cells and the card/region selectors are the ones person-cards-hosted-drill.mjs (Cloud apps/web/scripts) uses; they are copied here so this
// repo needs no Cloud checkout. Playwright comes from PW_FROM (a node_modules path that holds playwright-core); Chrome from CHARIOX_PC_CHROME.
// Typing is real keyboard events (no fill()), so the first-click and focus behaviour is what gets tested.
import { createRequire } from "node:module"
import { readFileSync } from "node:fs"

export const cells = [
  { id: "desktop-dpr1", width: 1440, height: 900, dpr: 1, isMobile: false },
  { id: "desktop-dpr2", width: 1440, height: 900, dpr: 2, isMobile: false },
  { id: "phone-dpr2", width: 390, height: 844, dpr: 2, isMobile: true },
]
// Selectors copied from the Cloud drill: card by interaction id, the kernel-browser surface in the card, prompt box.
export const sel = {
  card: "[data-interaction-id]",
  surface: ".kernel-browser-page-surface",
  paintMode: ".kernel-browser-page-surface[data-paint-mode=mirror],.kernel-browser-page-surface[data-paint-mode=video]",
  prompt: "[data-prompt-input]", send: "[data-action=send-prompt]",
}

const playwright = () => createRequire((process.env.PW_FROM ?? ".").replace(/\/?$/, "/") + "x.js")("playwright-core")

export async function launch({ headless = true } = {}) {
  return playwright().chromium.launch({ headless, executablePath: process.env.CHARIOX_PC_CHROME ?? "/usr/bin/google-chrome", args: ["--no-sandbox"] })
}

/**
 * Vendored replacement for the host-local sign-in helper (the Cloud drill imports /w/valenv/scripts/sign-in.mjs): the signed-in session is a
 * Playwright storageState JSON file the lane's operator made once, named by CHARIOX_PC_STORAGE_STATE. No password or token is read or typed here.
 */
export function storageStateFromEnv() {
  const file = process.env.CHARIOX_PC_STORAGE_STATE
  if (!file) return undefined
  JSON.parse(readFileSync(file, "utf8")) // fail early on a bad file; the content is never printed
  return file
}

/** One client of a cell. role: "owner" is the only one the cells use. */
export async function openClient(browser, cell, { storageState = storageStateFromEnv() } = {}) {
  const context = await browser.newContext({ viewport: { width: cell.width, height: cell.height }, deviceScaleFactor: cell.dpr, isMobile: cell.isMobile, hasTouch: cell.isMobile, storageState })
  const page = await context.newPage()
  const errors = []
  page.on("console", m => { if (m.type() === "error") errors.push(m.text().slice(0, 200)) })
  page.on("pageerror", e => errors.push(String(e.message).slice(0, 200)))
  return { context, page, cell, errors }
}

export const waitCard = (page, text, timeout = 60000) => page.locator(sel.card).filter({ hasText: text }).first().waitFor({ timeout })

/** Real key events into the focused element, after a real click on it: tests lost first clicks. */
export async function typeIn(page, locator, text) { await locator.click(); await page.keyboard.type(text, { delay: 20 }) }

/** Click at coordinates relative to an element (a projected region), as a person would. */
export async function clickIn(page, locator, { x, y }) { const box = await locator.boundingBox(); if (!box) throw new Error("no box"); await page.mouse.click(box.x + x, box.y + y) }

/**
 * Geometry (D3-9) and pixels (D3-5): box vs viewport, and the colour count of the element's screenshot, so "black" is a number.
 * The screenshot is decoded in a blank page of the same browser (no image library).
 */
export async function regionFacts(client, locator) {
  const { page } = client
  const box = await locator.boundingBox(), viewport = page.viewportSize()
  const png = await locator.screenshot()
  const probe = await client.context.newPage()
  try {
    const stats = await probe.evaluate(async b64 => {
      const bmp = await createImageBitmap(await (await fetch("data:image/png;base64," + b64)).blob())
      const c = new OffscreenCanvas(bmp.width, bmp.height), g = c.getContext("2d"); g.drawImage(bmp, 0, 0)
      const d = g.getImageData(0, 0, bmp.width, bmp.height).data, colours = new Set(); let dark = 0
      for (let i = 0; i < d.length; i += 4) { colours.add((d[i] << 16) | (d[i + 1] << 8) | d[i + 2]); if (d[i] + d[i + 1] + d[i + 2] < 30) dark++ }
      return { colours: colours.size, dark_share: dark / (d.length / 4), width: bmp.width, height: bmp.height }
    }, png.toString("base64"))
    return {
      cell: client.cell.id, box, viewport,
      whole: !!box && box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 0.5 && box.y + box.height <= viewport.height + 0.5,
      centred_x: !!box && Math.abs(box.x + box.width / 2 - viewport.width / 2) < viewport.width * 0.2,
      ...stats, drawn: stats.colours > 8 && stats.dark_share < 0.5,
    }
  } finally { await probe.close() }
}

/** Count of open cards over time across several clients (D3-8): returns {stop()} that yields the timeline. */
export function cardTimeline(clients, intervalMs = 250) {
  const rows = [], t0 = Date.now()
  const timer = setInterval(async () => {
    const counts = await Promise.all(clients.map(c => c.page.locator(sel.card).count().catch(() => -1)))
    rows.push({ t: Date.now() - t0, counts })
  }, intervalMs)
  return { stop: () => { clearInterval(timer); return rows } }
}
