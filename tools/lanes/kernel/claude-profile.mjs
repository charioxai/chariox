// MP-08 / MP-10: a labelled Claude account profile in the executor kernel, created or updated from a token file.
// A queue entry names it by label (`account`), loopd resolves the label to the profile id.
import { replaceClaudeCredential } from './claude-account.mjs';
import { syncWorkerAgents } from './worker-agents.mjs';

const LABEL = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

export function parseCredentialArgs(args) {
  const parsed = {};
  for (let i = 0; i < args.length; i += 2) {
    const name = args[i].slice(2), value = args[i + 1];
    if (!['from', 'profile'].includes(name) || !args[i].startsWith('--') || !value || value.startsWith('--') || name in parsed) return { error: `unexpected argument ${args[i]}` };
    parsed[name] = value;
  }
  if (parsed.profile !== undefined && !LABEL.test(parsed.profile)) return { error: `${parsed.profile} is not a profile label (letters, digits, . _ -)` };
  if (parsed.profile !== undefined && parsed.from === undefined) return { error: '--profile needs --from <token-file>: another profile never takes the default account token' };
  return parsed;
}

export async function storeClaudeProfile({ rpc, label, token, configDirectory, agentSource, agentNames }) {
  const profiles = async () => (await rpc({ ListProviderAccountProfiles: { provider: 'claude' } })).ProviderAccountProfilesListed.profiles.filter(p => p.label === label);
  let found = await profiles();
  if (found.some(p => p.origin !== 'chariox_created')) throw new Error(`label ${label} belongs to a linked profile; pick another label so queue entries name one profile`);
  if (found.length > 1) throw new Error(`${found.length} profiles are labelled ${label}; remove the extra ones in Provider Accounts`);
  const created = !found.length;
  if (created) {
    await rpc({ CreateProviderAccountProfile: { provider: 'claude', label } });
    found = await profiles();
  }
  if (found.length !== 1) throw new Error(`${label} profile was not created`);
  const profileId = found[0].profile_id;
  const auth = await replaceClaudeCredential({ rpc, profileId, token });
  // Workers use the profile's config directory as their HOME for Claude: the shared subagent definitions go there.
  const target = configDirectory(profileId);
  const agents = target ? syncWorkerAgents({ source: agentSource, target, names: agentNames }) : { reason: 'profile config directory not found' };
  return { claudeProfile: profileId, created, auth, agents };
}
