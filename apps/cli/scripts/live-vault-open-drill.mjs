// MP-08 / MP-09 / MP-10 / MP-11: real hosted new-UI Vault generation/fill.
// Prepare an unlocked disposable Vault and a real provider Room as a non-root
// kernel. VOPEN_EXPECT=requires-sudo runs the same creation step on the base.
import assert from 'node:assert/strict'
import {mkdir,writeFile} from 'node:fs/promises'
import {resolve} from 'node:path'
import {fileURLToPath} from 'node:url'

const {chromium}=await import(process.env.VOPEN_PLAYWRIGHT_MODULE ?? 'playwright-core')
const origin=process.env.VOPEN_ORIGIN, room=process.env.VOPEN_ROOM, output=process.env.VOPEN_EVIDENCE
assert.ok(origin && new URL(origin).protocol==='https:', 'VOPEN_ORIGIN must name the owned HTTPS stack')
assert.ok(room && output, 'VOPEN_ROOM and VOPEN_EVIDENCE are required')
const repo=resolve(fileURLToPath(new URL('../../..',import.meta.url)))
assert.ok(!resolve(output).startsWith(repo+'/'), 'evidence must stay outside the repository')
const expected=process.env.VOPEN_EXPECT ?? 'allowed'
assert.ok(['allowed','requires-sudo'].includes(expected))
const dpr=Number(process.env.VOPEN_DPR ?? 1), marker=`VOPEN_${Date.now()}`
await mkdir(output,{recursive:true})
let browser, page
const sockets=[], dialogs=[], checks=[]
try{
browser=await chromium.launch({headless:true,executablePath:process.env.VOPEN_CHROME ?? '/opt/google/chrome/chrome',
 args:['--no-sandbox','--disable-dev-shm-usage'],
 ...(process.env.SHAPE_PROXY?{proxy:{server:process.env.SHAPE_PROXY,bypass:''}}:{})})
const context=await browser.newContext({viewport:{width:1440,height:1000},deviceScaleFactor:dpr,
 recordVideo:{dir:output+'/video',size:{width:1440,height:1000}}})
page=await context.newPage()
page.on('websocket',socket=>sockets.push(new URL(socket.url()).origin))
page.on('dialog',async dialog=>{dialogs.push(dialog.type());await dialog.dismiss()})
const check=(name,ok)=>{checks.push({name,ok:Boolean(ok)});assert.ok(ok,name)}
const shot=name=>page.screenshot({path:output+'/'+name+'.png'})
async function relayOnly(){
 await page.keyboard.press('F8')
 await page.locator('[data-link-developer-tool]').waitFor()
 const choices=page.locator('[data-link-developer-tool] [data-carrier-mode=relay]')
 for(let i=0;i<await choices.count();i++)await choices.nth(i).click()
 await page.waitForTimeout(2500)
 const carriers=await page.locator('[data-link-developer-tool] [data-link-carrier]').evaluateAll(es=>es.map(e=>({
  carrier:e.getAttribute('data-link-carrier'), selected:e.querySelector('[data-carrier-mode=relay]')?.getAttribute('aria-pressed')})))
 check('real relay selected',carriers.length>0&&carriers.every(c=>c.selected==='true'&&['relay','none'].includes(c.carrier)))
 await page.getByRole('button',{name:'Close inspector',exact:true}).click()
 return carriers
}
async function submit(prompt,success,failure){
 const field=page.locator('[data-prompt-input]');await field.click();await field.fill(prompt);await field.press('Enter')
 await page.waitForFunction(({success,failure})=>[...document.querySelectorAll('.pane-transcript-reply')]
  .some(e=>e.innerText.includes(success)||e.innerText.includes(failure)),{success,failure},{timeout:240000})
 const replies=await page.locator('.pane-transcript-reply').allInnerTexts()
 check(success,replies.some(reply=>reply.includes(success))&&!replies.some(reply=>reply.includes(failure)))
 check('no owner interaction or sudo popup',await page.locator('[data-passkey-prompt],[data-interaction-id]').count()===0)
}
async function navigate(url){
 const omni=page.locator('.kernel-browser-omnibox').first();await omni.waitFor({timeout:60000})
 await omni.fill(url);await omni.press('Enter')
 await page.waitForFunction(host=>[...document.querySelectorAll('.desktop iframe')].some(f=>{
  try{return f.offsetWidth>0&&f.style.visibility!=='hidden'&&f.contentDocument?.body?.innerText.includes(host)}catch{return false}
 }),new URL(url).hostname==='github.com'?'Sign in to GitHub':'GitLab',{timeout:60000})
}
 await page.goto(origin+'/validation/ready')
 await page.goto(origin+'/terminal?ui=os',{waitUntil:'domcontentloaded'})
 await page.locator('[data-feature-new=room]').waitFor({state:'attached',timeout:60000})
 const open=page.locator(`[data-action=open-room][aria-label^="Open ${room}."]`)
 await open.waitFor({timeout:60000});await open.click();await page.waitForTimeout(700)
 if(!await page.locator('.kernel-browser-window').count()){
  await page.locator('[data-action=open-room-view]').last().click()
  await page.locator('[data-open-room-view=browser]').click()
 }
 const carriers=await relayOnly()
 await navigate('https://github.com/login');await shot('before-generation')
 if(expected==='requires-sudo'){
  await submit(`MP-08 / MP-09 / MP-10 / MP-11 regression drill: call chariox.vault.generate exactly once with request_id ${marker} and origin https://github.com. Do not request sudo, approval, a plan, or any other tool. If that call is refused because it needs a sudo window, finish with exactly ${marker}_REQUIRES_SUDO. Otherwise finish with exactly ${marker}_UNEXPECTED. Never print or read secret values.`,marker+'_REQUIRES_SUDO',marker+'_UNEXPECTED')
  await shot('base-sudo-refusal')
 }else{
  await submit(`MP-08 / MP-09 / MP-10 / MP-11 live drill: generate a password with chariox.vault.generate, request_id ${marker}, origin https://github.com. Retry that same request once and verify the same credential_id. Use the Chariox kernel Browser tools on the existing GitHub sign-in tab: get a fresh snapshot, fill only the Password input with that handle using chariox.kernel_browser_paste_secret and submit=false. Do not sign in, submit, request sudo, approval, a plan, or read/copy/re-display any secret. Get a fresh model snapshot and screenshot to verify the password stays masked. Remember the credential_id for the next turn. Finish with exactly ${marker}_FILLED if both generation and fill succeeded; otherwise exactly ${marker}_FAILED.`,marker+'_FILLED',marker+'_FAILED')
  const fields=await page.evaluate(()=>[...document.querySelectorAll('.desktop iframe')].filter(f=>f.offsetWidth>0&&f.style.visibility!=='hidden').map(f=>{
   const input=f.contentDocument?.querySelector('#password');return {found:!!input,type:input?.type,
    protected:input?.getAttribute('aria-label')==='Protected content',safeValue:!input?.value||/^[•●*]+$/.test(input.value)}
  }))
  check('mirrored password value remains protected',fields.some(f=>f.found&&(f.type==='password'||f.protected)&&f.safeValue))
  await writeFile(output+'/field-protection.json',JSON.stringify(fields,null,2));await shot('generated-and-filled')
  await navigate('https://gitlab.com/users/sign_in');await shot('other-origin-before-use')
  await submit(`MP-08 / MP-09 / MP-10 / MP-11: reuse the exact credential_id generated in the previous turn, without generating or changing an entry. Get a fresh snapshot of the current GitLab sign-in tab and attempt chariox.kernel_browser_paste_secret into its Password input with submit=false. This must be refused because the entry belongs to https://github.com. Do not submit, sign in, request sudo or approvals, or inspect/copy a secret. Finish with exactly ${marker}_OTHER_ORIGIN_REFUSED only if the actual fill tool refuses the origin; otherwise exactly ${marker}_WRONG_ORIGIN_FAILED.`,marker+'_OTHER_ORIGIN_REFUSED',marker+'_WRONG_ORIGIN_FAILED')
  await shot('other-origin-refused')
 }
 check('encrypted hosted WebSocket used',sockets.some(url=>url.startsWith('wss://')))
 check('no browser dialogs',dialogs.length===0)
 await writeFile(output+'/result.json',JSON.stringify({expected,dpr,room,marker,checks,sockets,carriers,dialogs},null,2))
 console.log('PASS',expected,room,'DPR'+dpr)
}catch(error){
 await page?.screenshot({path:output+'/failure.png'}).catch(()=>{})
 await writeFile(output+'/result.json',JSON.stringify({expected,dpr,room,marker,checks,sockets,dialogs,error:error.message},null,2))
 throw error
}finally{await browser?.close()}
