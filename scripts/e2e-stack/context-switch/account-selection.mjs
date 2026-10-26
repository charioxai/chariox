// MP-08 / MP-10 / MP-11: CLI aliases are labels; kernel acknowledgements use IDs.
export function accountSelectionCommand({agentId,profileId,profiles}){
 const label=profiles?.find(profile=>profile.profile_id===profileId)?.label;
 if(typeof label!=='string'||!label.trim()||/[\r\n\0]/.test(label))throw Error('A safe registered account label is required');
 return `agent account ${agentId} ${label}`;
}
