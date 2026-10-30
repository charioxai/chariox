import assert from 'node:assert/strict'
import { mkdirSync, writeFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

export async function startProviderLoad({ rpc, root, profilePath, count, objects = 1000, report, save }) {
  assert(profilePath, 'existing product-linked Codex profile path required')
  const linked = await rpc({ LinkProviderAccountProfile: { provider: 'codex', label: 'responsiveness', path: profilePath } })
  const profile = linked.ProviderAccountProfile.profile.profile_id
  const agents = []
  report.provider_load = { profile_id: profile, model: 'gpt-6-luna', effort: 'low', objects_per_agent: objects, agents }; save()
  for (let i = 0; i < count; i++) {
    const work = join(root, `build-${i}`)
    mkdirSync(work)
    let source = '#define JOIN_(a,b) a##b\n#define JOIN(a,b) JOIN_(a,b)\n#define NAME(n) JOIN(JOIN(f_, INDEX), n)\n'
    for (let n = 0; n < 64; n++) source += `unsigned NAME(_${n})(unsigned x) { return ((x*${n+1}u)^0x9e3779b9u)+(x>>3); }\n`
    writeFileSync(join(work, 'source.c'), source)
    // Bound compiler duty while keeping a real build active over the measurement.
    writeFileSync(join(work, 'Makefile'), `OBJECTS := $(addprefix object,$(addsuffix .o,$(shell seq 1 ${objects})))\n.PHONY: test\ntest: $(OBJECTS)\n\t@echo BUILD_OK\nobject%.o: source.c\n\t@cc -O2 -DINDEX=$* -c $< -o $@\n\t@sleep 0.5\n`)
    const created = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work,
      agent_defaults: { provider: 'codex', account_profile: profile, model: 'gpt-6-luna', effort: 'low', permission_level: 'yolo' } } })).SessionCreated
    const sessionId = created.session.id, agentId = created.agent.id
    const attachment = (await rpc({ AttachToSession: { session_id: sessionId, client_id: `load-${i}`, capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
    const at = performance.now()
    const launched = await rpc({ LaunchProviderRun: { session_id: sessionId, agent_id: agentId, adapter_key: 'codex', provider: 'codex', account_profile: profile, model: 'gpt-6-luna', variant: 'low', native_tui: false } })
    const run = launched.ProviderRunLaunched?.provider_run ?? launched.ProviderRunLaunchAccepted?.provider_run ?? launched.ProviderRun?.provider_run
    agents.push({ session_id: sessionId, agent_id: agentId, launch_ms: performance.now()-at, launch_response: Object.keys(launched)[0], provider_run: run })
    save()
    await rpc({ SubmitPrompt: { session_id: sessionId, attachment_id: attachment, target_agent_id: agentId,
      prompt: 'Run make -j1 test in the current directory. Wait for it to finish and reply exactly BUILD_OK. Do not read unrelated files or credentials.', attachments: [] } })
  }
  for (let attempt = 0; ; attempt++) {
    const builds = Array.from({ length: count }, (_, i) => readdirSync(join(root, `build-${i}`)).filter(name => name.endsWith('.o')).length)
    report.provider_load.compiled_objects = builds; save()
    if (builds.every(count => count > 0)) break
    assert(attempt < 180, 'official provider turns did not begin their builds within 90 seconds')
    await delay(500)
  }
  return {
    async stop() {
      for (const agent of agents) await rpc({ EndSession: { session_id: agent.session_id } })
    },
    async status() {
      const result = []
      for (const agent of agents) {
        const session = (await rpc({ GetSessionState: { session_id: agent.session_id } })).SessionState
        const runId = session.session.active_provider_run_id ?? Object.values(session.agent_activity ?? {}).find(activity => activity.active_turn)?.active_turn.provider_run_id
        const providerRun = runId ? (await rpc({ GetProviderRun: { provider_run_id: runId } })).ProviderRun?.provider_run : null
        result.push({ session_id: agent.session_id, activity: session.agent_activity, agents: session.session.agents, provider_run: providerRun })
      }
      return result
    },
  }
}
