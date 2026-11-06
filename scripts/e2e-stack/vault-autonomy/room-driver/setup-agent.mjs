// MP-08 / MP-09 / MP-10 / MP-11: link a provider profile directory (readable by the kernel user) and spawn
// one Room agent on it: node setup-agent.mjs <codex|claude|opencode> <profile-dir> <alias> <model> <effort> <worktree>.
import {connect} from './rpc.mjs'

const sessionId = process.env.ROOM_SESSION
const [provider, path, alias, model, effort, worktree] = process.argv.slice(2)
if (!sessionId || !worktree) throw Error('Set ROOM_SESSION and pass: provider profile-dir alias model effort worktree')
const peer = await connect()
const profile = (await peer.send({LinkProviderAccountProfile: {provider, label: `${alias} drill`, path}})).ProviderAccountProfile.profile
const agent = (await peer.send({SpawnAgent: {session_id: sessionId, alias, provider, account_profile: profile.profile_id, model, effort, worktree_id: worktree}})).AgentSpawned.agent
console.log(JSON.stringify({profile: profile.profile_id, agent: agent.id}))
peer.close()
