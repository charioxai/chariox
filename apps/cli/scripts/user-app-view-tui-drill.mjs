#!/usr/bin/env bun
// Real PTY/OpenTUI projection and production LocalIpcClient control transport.
// The loopback kernel peer is synthetic; real host/authority is validated by
// multidomain-host-drill.mjs. No Session/provider/account is created here.
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const cli = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
if (process.argv[2] === '--terminal') {
  const { createCliRenderer, TextRenderable } = await import('@opentui/core')
  const { createRoot } = await import('solid-js')
  const { LocalIpcClient } = await import('../dist/ipc.js')
  const { createCliUserAppViewsComposition } = await import('../dist/cli-user-app-views-composition.js')
  const { handleAppSlashCommand } = await import('../dist/app-command-handler.js')
  const { parseSlashCommand } = await import('../dist/commands.js')
  const client = new LocalIpcClient(process.argv[3])
  const renderer = await createCliRenderer({ exitOnCtrlC:false,useThread:false,useConsole:false, useKittyKeyboard:null })
  const status = new TextRenderable(renderer,{content:'Idle controls: l list / o open / s show / c call / a approvals / d deny / q exit', position:'absolute', top:27})
  renderer.root.add(status)
  let dispose
  const views = createRoot(cleanup=>{dispose=cleanup;return createCliUserAppViewsComposition({client:()=>client,renderer,
    dimensions:()=>({width:110,height:28}),themeRevision:()=>0,currentFocus:()=>null,promptFocus:()=>null,
    notify:text=>{status.content=text},approvalOwnsInput:()=>false,
  })})
  if(!views) throw Error('flagged projection unavailable')
  const commands={l:'/app views',o:'/app open fixture',s:'/app view show fixture-view',c:`/app view call echo '{"text":"keyboard command"}'`,a:'/app view approvals',d:'/app view answer fixture-approval deny'}
  let pending=Promise.resolve()
  renderer.keyInput.on('keypress',event=>{
    if(views.handleKey(event)) return
    if(event.name==='q') {void pending.finally(async()=>{dispose();await views.dispose();await client.close();renderer.destroy();process.exit(0)});return}
    const command=commands[event.name]
    if(command) pending=pending.then(()=>handleAppSlashCommand({userAppViews:views,sendAppRequest:r=>client.send(r),appendNotice:text=>{status.content=text},flashFooter:text=>{status.content=text}},parseSlashCommand(command))).catch(()=>{status.content='Projection command failed'})
  })
  const frameTimer=setInterval(()=>{void writeFile(process.argv[4],new TextDecoder().decode(renderer.currentRenderBuffer.getRealCharBytes(true)))},100)
  renderer.on('destroy',()=>clearInterval(frameTimer))
  renderer.start()
} else {
  const { WebSocketServer } = await import('ws')
  const evidence=path.resolve(process.argv[2]??path.join(tmpdir(),'cx-app-tui-evidence'))
  assert.ok(evidence!==cli && !evidence.startsWith(cli+path.sep),'evidence must be outside repository')
  await mkdir(evidence,{recursive:true})
  const scratch=await mkdtemp(path.join(tmpdir(),'cx-app-tui-'))
  const server=new WebSocketServer({host:'127.0.0.1',port:0});await new Promise(resolve=>server.once('listening',resolve))
  let views=[],value='',focused=false,calls=0,approval=true,output=''
  const requests=[],checks=[]
  const view={view_id:'fixture-view',installation_id:'fixture',generation:'fixture-generation',origin:'https://fixture.invalid',browser:{tab_id:'host-tab-fixture',generation:7}}
  server.on('connection',socket=>socket.on('message',bytes=>{
    const frame=JSON.parse(String(bytes)),request=frame.request;requests.push(request)
    let response
    if(request.OpenUserAppView){assert.deepEqual(request.OpenUserAppView,{installation_id:'fixture',host:'kernel_browser'});views=[view];response={UserAppViewOpened:{view}}}
    else if(request.ListUserAppViews)response={UserAppViewsListed:{views}}
    else if(request.CloseUserAppView){views=[];response={UserAppViewClosed:{view_id:'fixture-view'}}}
    else if(request.CallUserAppView){calls++;response={UserAppViewCallResult:{result:{text:request.CallUserAppView.input.text},error:null}}}
    else if(request.SubscribeUserAppViews)response={UserAppViewsChanged:{cursor:1,views,interactions:approval?[{id:'fixture-approval',level:'warning',kind:'permission',title:'Fixture permission',message:'Allow fixture operation?',choices:[{id:'deny',label:'Deny',reply:'deny'}]}]:[]}}
    else if(request.AnswerUserDomainInteraction){assert.equal(request.AnswerUserDomainInteraction.choice_id,'deny');assert.equal(request.AnswerUserDomainInteraction.passkey,null);approval=false;response={UserDomainInteractionAnswered:{interaction_id:'fixture-approval'}}}
    else if(request.KernelBrowser){
      const command=request.KernelBrowser.command;assert.equal(command.tab_id,view.browser.tab_id);assert.equal(command.generation,7)
      if(command.op==='input'){
        if(command.input.kind==='key'&&command.input.key==='Tab')focused=true
        if(command.input.kind==='text'&&focused)value+=command.input.text
        if(command.input.kind==='key'&&command.input.key==='Enter'){calls++}
      }
      response={KernelBrowser:{result:{generation:7,snapshot:{accessibility_nodes:[{node_ref:'root',role:'heading',name:'Fixture App text'},
        {node_ref:'field',parent_ref:'root',role:'textbox',name:'Message',value,focused},
        {node_ref:'result',role:'StaticText',name:calls?'App channel reply received':'Waiting for keyboard App action'}]}}}}
    }else throw Error('unexpected fixture request '+JSON.stringify(request))
    socket.send(JSON.stringify({type:'response',request_id:frame.request_id,response,error:null}))
  }))
  const terminal=Bun.spawn([process.execPath,'--conditions=browser',fileURLToPath(import.meta.url),'--terminal',`ws://127.0.0.1:${server.address().port}`,path.join(evidence,'frame.txt')],{
    cwd:cli,env:{...process.env,CHARIOX_USER_APP_VIEWS_PROTOTYPE:'1',CHARIOX_HOME:scratch,CHARIOX_LOG_DIR:scratch,TERM:'xterm-256color'},
    terminal:{cols:110,rows:28,data(_terminal,bytes){output+=new TextDecoder().decode(bytes)}},
  })
  const strip=text=>text.replace(/\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[=>78]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g,'')
  const wait=async(predicate,label)=>{const until=Date.now()+15000;while(!predicate()){if(Date.now()>until)throw Error('timeout: '+label);await Bun.sleep(50)}}
  const press=async(bytes)=>{terminal.terminal.write(bytes);await Bun.sleep(250)}
  const check=name=>{checks.push({name,result:'PASS'});console.log(name+': PASS')}
  try{
    await wait(()=>strip(output).includes('Idle controls'),'terminal startup');await press('l');await wait(()=>strip(output).includes('No open views'),'list');check('list without Session')
    await press('\x1b');await press('o');await wait(()=>strip(output).includes('Fixture App text'),'projection');assert.equal(views.length,1);check('open kernel-hosted user view and render accessibility text')
    await press('\t');await press('typed in terminal');await press('\r');await wait(()=>calls===1,'App reply');assert.equal(value,'typed in terminal');await Bun.sleep(300);const frame=await Bun.file(path.join(evidence,'frame.txt')).text();assert.match(frame,/App channel reply received/);await writeFile(path.join(evidence,'app-frame.txt'),frame);check('real terminal Tab/type/Enter App channel action')
    await press('\x1b');await press('c');await wait(()=>calls===2,'direct channel');check('keyboard command uses bound CallUserAppView')
    await press('a');await wait(()=>strip(output).includes('Allow fixture operation?'),'approval');check('trusted kernel approval text')
    await press('\x1b');await press('d');await wait(()=>!approval,'approval reply');check('owner-scoped approval choice via keyboard command')
    await press('s');await press('\x17');await wait(()=>views.length===0,'close');check('Ctrl+W explicitly closes user view')
    await press('l');assert.match(await Bun.file(path.join(evidence,'frame.txt')).text(),/No open views/);check('list after close');await press('\x1b');await press('q')
    await wait(()=>terminal.exitCode!==null,'terminal exit');assert.equal(terminal.exitCode,0)
    assert.ok(requests.every(r=>!JSON.stringify(r).includes('session_id')));check('no Room or session requests')
    await writeFile(path.join(evidence,'terminal.txt'),strip(output));await writeFile(path.join(evidence,'wire-requests.json'),JSON.stringify(requests,null,2))
    await writeFile(path.join(evidence,'results.json'),JSON.stringify({scope:'Real PTY/OpenTUI production projection and LocalIpcClient, synthetic loopback kernel/App peer. No live kernel, Chromium, provider or Vault claim; real host drill is separate.',checks},null,2))
  }finally{
    await writeFile(path.join(evidence,'terminal.txt'),strip(output));await writeFile(path.join(evidence,'wire-requests.json'),JSON.stringify(requests,null,2))
    terminal.kill();for(const socket of server.clients)socket.terminate();await new Promise(resolve=>server.close(resolve));await rm(scratch,{recursive:true,force:true})
  }
}
