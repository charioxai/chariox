// MP-08 / MP-10: a queue entry names its Claude account by profile label; the executor kernel says which profile that is.
import { LaneError } from '../lanes.mjs';

const unavailable = message => new LaneError('account_unavailable', `account: ${message}`);

export async function claudeProfileFor({ cli, kernel, label }) {
  const answer = await cli(['request', '--kernel', kernel], JSON.stringify({ ListProviderAccountProfiles: { provider: 'claude' } }));
  const listed = answer?.ProviderAccountProfilesListed?.profiles;
  if (!Array.isArray(listed)) throw new LaneError('unexpected_response', 'the kernel did not list its Claude profiles; check the executor kernel.');
  const claude = listed.filter(profile => profile.provider === 'claude');
  const matches = claude.filter(profile => profile.label === label);
  const fix = `lanes-kernel.mjs set-claude-credential --profile ${label} --from <token-file>`;
  if (!matches.length) throw unavailable(`no Claude profile is labelled ${label}; known labels: ${claude.map(profile => profile.label).join(', ') || 'none'}. Create it with ${fix}`);
  if (matches.length > 1) throw unavailable(`${matches.length} Claude profiles are labelled ${label}; give them distinct labels in Provider Accounts.`);
  const [profile] = matches;
  if (profile.auth_state !== 'authenticated') throw unavailable(`${label} is ${profile.auth_state}; run ${fix}`);
  return profile.profile_id;
}
