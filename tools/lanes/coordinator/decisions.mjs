// MP-08 / MP-09 / MP-10 / MP-11: which provider decisions loopd answers by itself and which belong to the coordinator.
const OWNER_WORDS = /passkey|credential|vault|secret|token|password|payment|\bsudo\b|log ?in\b|api key/i;

// A decision is routine only when the kernel says it is an ordinary tool permission and offers a plain allow. Anything
// critical, tied to an owner hand-off, mentioning credentials, or not readable at all goes to the coordinator with the reason.
export function classifyDecision(interaction) {
  if (!interaction) return { routine: false, reason: 'unclassified' };
  if (interaction.level === 'critical') return { routine: false, reason: 'critical' };
  if (interaction.ownerOnly) return { routine: false, reason: 'owner_only' };
  if (interaction.kind !== 'permission') return { routine: false, reason: 'question' };
  const title = String(interaction.title ?? '');
  if (OWNER_WORDS.test(title)) return { routine: false, reason: 'credential_related' };
  if (!/^Approve Claude Code /.test(title)) return { routine: false, reason: 'not_a_claude_permission' };
  if (!(interaction.choices ?? []).some(choice => choice === 'allow_once' || choice === 'allow_session')) return { routine: false, reason: 'no_allow_choice' };
  return { routine: true };
}
