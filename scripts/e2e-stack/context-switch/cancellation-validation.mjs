// MP-08 / MP-10 / MP-11: cancellation observation used by the real fault drill.
export async function verifyCancelledPromptRecovery({settle,wait,snapshot,checkFold,recover,agentId,marker}){
 function check(state){
  const rows=state.agentPanes?.[agentId];
  if(!Array.isArray(rows))throw Error('Cancelled agent pane is missing');
  if(rows.some(row=>row.role==='assistant'&&row.text?.includes(marker)))throw Error('Cancelled prompt received a provider answer');
 }
 await settle();
 await wait(8000);
 check(await snapshot());
 await checkFold();
 try{await recover();}finally{check(await snapshot());}
 return {cancelledAnswerAbsent:true,cancelledFoldAbsent:true,recoveryRecall:9};
}
