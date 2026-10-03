// MP-08/MP-10/MP-11: queued promotion changes the admission ID.
export function completedDrillTurn(turns, acceptedId, outcome, prompt, baseline) {
 const matches = turns.filter(turn => turn.lifecycle === 'completed'
  && !baseline.has(turn.prompt_id)
  && (outcome === 'Queued'
   ? turn.entries?.some(({entry}) => entry?.kind === 'user_prompt' && entry.text === prompt)
   : turn.prompt_id === acceptedId));
 if (matches.length > 1) throw new Error('duplicate completed drill turn');
 return matches[0];
}
