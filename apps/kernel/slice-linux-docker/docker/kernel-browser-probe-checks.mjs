// MP-08/MP-10/MP-11: external checks of the kernel browser against public bot-detection and login pages. Each check
// returns a short normalised outcome; the probe compares it with the outcome of a stock Chrome of the same build.
import { setTimeout as delay } from 'node:timers/promises';

const blockedText = /access denied|you have been blocked|attention required|unusual traffic|just a moment|verify you are human|are you a robot|captcha|couldn.t sign you in|may not be secure/i;
const text = page => page.evaluate('document.body ? document.body.innerText : ""');
const classify = (url, body) => blockedText.test(body) ? 'blocked-or-challenged' : 'loaded';

async function loginPage(page, url, field, wait = 8) {
  await page.navigate(url, wait * 1000);
  const body = await text(page);
  const hasField = field ? await page.evaluate(`!!document.querySelector(${JSON.stringify(field)})`) : null;
  return { outcome: `${classify(url, body)}${hasField === null ? '' : hasField ? ' first-field-present' : ' first-field-missing'}`, detail: (await page.evaluate('document.title')).slice(0, 80) };
}

export const googleEmail = 'chariox.browsercheck.nobody.847261@gmail.com'; // does not exist; never a real account, never a password

export const checks = [
  { id: 'google-email-step', title: 'Google sign-in, email step (throwaway address)', external: true, async run(page) {
    await page.navigate('https://accounts.google.com/signin/v2/identifier?service=mail&flowName=GlifWebSignIn&flowEntry=ServiceLogin', 5000);
    for (let i = 0; i < 20 && !(await page.evaluate("!!document.querySelector('#identifierId')")); i++) await delay(500);
    await delay(1500);
    await page.evaluate("document.querySelector('#identifierId')?.focus()");
    await page.type(googleEmail);
    const {x, y} = JSON.parse(await page.evaluate("(() => { const b = document.querySelector('#identifierNext button') || [...document.querySelectorAll('button')].find(x => /next/i.test(x.innerText)); const r = b.getBoundingClientRect(); return JSON.stringify({x: r.x + r.width / 2, y: r.y + r.height / 2}); })()"));
    await page.click(x, y);
    await delay(7000);
    const url = await page.url(), body = await text(page);
    // rejected = the browser is refused; captcha = Google doubts the address/IP (not the browser); found-no-account = accepted
    const outcome = /signin\/rejected/.test(url) || /may not be secure/i.test(body) ? 'rejected' : /find this account/i.test(body) ? 'accepted (no such account)' : /Type the text you hear or see/i.test(body) ? 'captcha' : /password/i.test(body) ? 'accepted (password step)' : 'unknown';
    return { outcome, detail: url.slice(0, 80) };
  } },
  { id: 'browserscan', title: 'BrowserScan bot detection', external: true, async run(page) {
    await page.navigate('https://www.browserscan.net/bot-detection', 14000);
    const body = (await text(page)).replace(/\s+/g, ' ');
    const result = /Test Results:\s*(\w+)/.exec(body)?.[1] ?? 'no result';
    return { outcome: result, detail: body.slice(body.indexOf('Test Results'), body.indexOf('Test Results') + 80) };
  } },
  { id: 'sannysoft', title: 'bot.sannysoft.com failed rows', external: true, async run(page) {
    await page.navigate('https://bot.sannysoft.com/', 6000);
    const failed = JSON.parse(await page.evaluate("JSON.stringify([...document.querySelectorAll('tr')].filter(r => /failed/.test(r.innerHTML)).map(r => r.cells[0].innerText.trim()))"));
    // A host without a GPU has no WebGL in stock Chrome and SwiftShader in the kernel browser: not a bot signal.
    const bot = failed.filter(name => !/^WebGL/.test(name));
    return { outcome: bot.join(', ') || 'none', detail: failed.filter(name => /^WebGL/.test(name)).join(', ') };
  } },
  { id: 'intoli', title: 'Intoli headless test failed rows', external: true, async run(page) {
    await page.navigate('https://intoli.com/blog/not-possible-to-block-chrome-headless/chrome-headless-test.html', 6000);
    const failed = JSON.parse(await page.evaluate("JSON.stringify([...document.querySelectorAll('tr')].filter(r => /failed/.test(r.innerHTML)).map(r => r.cells[0].innerText.trim()))"));
    return { outcome: failed.join(', ') || 'none', detail: '' };
  } },
  { id: 'turnstile', title: 'Cloudflare Turnstile demo token', external: true, async run(page) {
    await page.navigate('https://demo.turnstile.workers.dev/', 10000);
    const n = await page.evaluate("[...document.querySelectorAll('[name=cf-turnstile-response]')].reduce((a, i) => a + i.value.length, 0)");
    return { outcome: n > 0 ? 'token issued' : 'no token', detail: '' };
  } },
  { id: 'recaptcha-v3', title: 'reCAPTCHA v3 demo score', external: true, async run(page) {
    await page.navigate('https://recaptcha-demo.appspot.com/recaptcha-v3-request-scores.php', 12000);
    const score = /"score":\s*([0-9.]+)/.exec(await text(page))?.[1];
    return { outcome: score === undefined ? 'no score' : Number(score) >= 0.7 ? 'high score' : `low score ${score}`, detail: `score ${score}` };
  } },
  { id: 'cloudflare-site', title: 'Cloudflare-protected page', external: true, async run(page) {
    await page.navigate('https://nowsecure.nl/', 12000);
    return { outcome: classify('', await text(page)), detail: await page.evaluate('document.title') };
  } },
  { id: 'github-login', title: 'GitHub login first field', external: true, run: page => loginPage(page, 'https://github.com/login', '#login_field') },
  { id: 'microsoft-login', title: 'Microsoft login first field', external: true, run: page => loginPage(page, 'https://login.live.com/', 'input[type=email], input[name=loginfmt]') },
  { id: 'x-login', title: 'X login page', external: true, run: page => loginPage(page, 'https://x.com/i/flow/login', null, 10) },
  { id: 'apple-login', title: 'Apple ID sign-in page', external: true, run: page => loginPage(page, 'https://appleid.apple.com/sign-in', null, 8) },
  { id: 'stripe-checkout', title: 'Stripe checkout demo page load', external: true, run: page => loginPage(page, 'https://checkout.stripe.dev/', null, 8) },
];
