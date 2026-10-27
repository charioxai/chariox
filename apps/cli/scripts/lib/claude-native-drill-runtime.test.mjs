// MP-08 / MP-10 / MP-11: native drills use only the public provider/history projections.
import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp, mkdir, writeFile, rm} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {claudeNativeRunPaths, waitForClaudeNativePrompts} from './claude-native-drill-runtime.mjs'

test('native prompt evidence uses public kernel history without provider environments', async () => {
 const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-native-drill-test-'))
 try {
  const banner = path.join(root, 'banner.txt')
  await writeFile(banner, 'provider run: run-1\n')
  const client = {send: async request => {
   if (request.GetProviderRun) return {ProviderRun:{provider_run:{id:'run-1',session_id:'session-1',agent_instance_id:'agent-1'}}}
   assert.deepEqual(request, {GetSessionHistoryOutline:{session_id:'session-1',agent_ids:['agent-1'],latest_prompt_count:100}})
   return {SessionHistoryOutline:{agents:[{agent_id:'agent-1',turns:[{user_prompt:{entry:{text:'visible-native-request'}}}]}]}}
  }}
  const source = await claudeNativeRunPaths(client,banner)
  assert.deepEqual(await waitForClaudeNativePrompts(source,['visible-native-request'],1000), ['visible-native-request'])
 } finally {await rm(root,{recursive:true,force:true})}
})

// MP-08 / MP-10 / MP-11: kernel history can stay clean while the provider prompt leaks.
test('hidden-context check sees Claude transcript text independently of clean kernel history', async () => {
 const root = await mkdtemp(path.join(os.tmpdir(), 'chariox-native-transcript-test-'))
 const originalConfig = process.env.CLAUDE_CONFIG_DIR
 process.env.CLAUDE_CONFIG_DIR = root
 try {
  const banner = path.join(root, 'banner.txt')
  await writeFile(banner, 'provider run: run-1\n')
  const project = path.join(root, 'projects', '-work-repo')
  await mkdir(project, {recursive:true})
  await writeFile(path.join(project, 'claude-session.jsonl'), JSON.stringify({type:'user',sessionId:'claude-session',message:{role:'user',content:[{type:'text',text:'visible-native-request CHARIOX_NATIVE_TUI_HIDDEN_INSTRUCTIONS'},{type:'tool_result',content:'tool text'}]}})+'\n')
  const client = {send: async request => {
   if (request.GetProviderRun) return {ProviderRun:{provider_run:{id:'run-1',session_id:'session-1',agent_instance_id:'agent-1',provider_session_id:'claude-session',working_directory:'/work/repo'}}}
   return {SessionHistoryOutline:{agents:[{turns:[{user_prompt:{entry:{text:'visible-native-request'}}}]}]}}
  }}
  const source = await claudeNativeRunPaths(client,banner)
  // This is the exact input to the injection drill's hidden-instructions assertion.
  const prompts = await waitForClaudeNativePrompts(source,['visible-native-request'],1000, {providerTranscript:true})
  assert.deepEqual(prompts, ['visible-native-request CHARIOX_NATIVE_TUI_HIDDEN_INSTRUCTIONS'])
 } finally {
  if (originalConfig === undefined) delete process.env.CLAUDE_CONFIG_DIR
  else process.env.CLAUDE_CONFIG_DIR = originalConfig
  await rm(root,{recursive:true,force:true})
 }
})
