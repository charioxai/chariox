// MP-08 / MP-09 / MP-10 / MP-11: observe the existing hosted Room at both display densities.
import {writeFile, readFile} from 'node:fs/promises';
import {origin, out, record, sha, webContext} from './workspace-runtime.mjs';
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
export async function captureWeb(runtime, sessionId, agentId, label) {
    const results = [];
    for (const dpr of [1, 2]) {
        const context = await webContext({viewport: {width: 1440, height: 900}, deviceScaleFactor: dpr});
        const page = await context.newPage();
        try {
            const ready = await page.goto(origin + '/validation/ready');
            if (ready.status() !== 200) throw Error('Hosted validation session is unavailable');
            await page.goto(origin + '/waiting-room');
            await page.getByText('Waiting Room Ready', {exact: true}).waitFor({timeout: 30000});
            for (const [key, id] of [['machine', runtime.status.machine_id], ['kernel', runtime.status.daemon_id]]) {
                await page.locator(`[data-waiting-row-key="${key}"]`).click();
                await page.locator(`[data-option-picker-value="${id}"]`).click({timeout: 30000});
                await sleep(1000);
            }
            await page.locator('[data-waiting-row-key="join"]').click();
            const row = page.locator(`[data-session-picker-session-id="${sessionId}"]`);
            await row.waitFor({timeout: 30000});
            const bounds = await row.boundingBox();
            await row.dblclick({position: {x: Math.max(60, bounds.width * .55), y: bounds.height / 2}});
            await page.locator('[data-terminal-prompt]').waitFor({timeout: 30000});
            await page.locator(`.freeform-agent-pane[data-agent-id="${agentId}"]`).waitFor({timeout: 30000});
            await sleep(1000);
            const path = out + '/' + label + '-web-dpr' + dpr;
            await page.screenshot({path: path + '.png', fullPage: true});
            await writeFile(path + '.visible.txt', await page.locator('body').innerText(), {mode: 0o600});
            const scripts = await page.evaluate(async () => Promise.all([...document.querySelectorAll('script[src]')]
                .map(script => script.src).filter(url => new URL(url).origin === location.origin)
                .map(async url => { const bytes = await (await fetch(url)).arrayBuffer(); const hash = await crypto.subtle.digest('SHA-256', bytes); return {path: new URL(url).pathname, bytes: bytes.byteLength, sha256: [...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, '0')).join('')}; })));
            const result = {result: 'PASS', dpr, sessionId, agentId, screenshotSha256: sha(await readFile(path + '.png')), scripts};
            await record(label + '-web-dpr' + dpr, result);
            results.push(result);
        } catch (error) {
            const result = {result: 'FAIL', dpr, sessionId, agentId, reason: String(error.message ?? error)};
            await record(label + '-web-dpr' + dpr, result);
            results.push(result);
        } finally { await context.close(); }
    }
    return results;
}
