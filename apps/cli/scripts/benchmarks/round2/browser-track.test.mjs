// MP-08/MP-10/MP-11: audit policy failures without changing reward or tool policy.
import test from 'node:test'
import assert from 'node:assert/strict'
import { permittedBrowserTool, navigationAudit } from './browser-track.mjs'
const success = {tool:'slice_open_url',status:'completed',input:{url:'https://example.test/'},output:{browser:{action_kind:'navigate',url:'https://example.test/'}}}
const rejection = {tool:'slice_open_url',status:'error',input:{url:'chrome://downloads/'},error:'browser navigation URL must use HTTP or HTTPS'}
const actions = [{kind:'navigate',state:'completed'},{kind:'navigate',state:'failed'}]
test('MP-08/MP-10/MP-11 bounded first-party interaction stays on the Browser track', () => {
  assert.equal(permittedBrowserTool('slice_browser_interact'),true)
  assert.equal(permittedBrowserTool('slice_open_url'),false)
  assert.equal(permittedBrowserTool('slice_open_url',{allowNavigation:true}),true)
  for (const name of ['slice_computer_input','shell','slice_browser_eval','slice_browser_unknown']) assert.equal(permittedBrowserTool(name,{allowNavigation:true}),false)
})
test('MP-08/MP-10/MP-11 proven URL policy rejection after successful navigation is an agent failure', () => {
  assert.deepEqual(navigationAudit(actions,[success,rejection]),{firstFailingSeam:null,agentNavigationPolicyRejections:1})
})
test('MP-08/MP-10/MP-11 uncertain navigation failures remain invalid', () => {
  for (const trace of [[rejection],[success,{...rejection,input:{url:'https://example.test/'}}],[success,{...rejection,error:'browser_cdp_timeout'}]])
    assert.equal(navigationAudit(actions,trace).firstFailingSeam,'benchmark_navigation')
  assert.equal(navigationAudit([...actions,{kind:'navigate',state:'failed'}],[success,rejection]).firstFailingSeam,'benchmark_navigation')
})
