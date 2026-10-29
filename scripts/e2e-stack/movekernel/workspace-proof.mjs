// MP-08 / MP-09 / MP-10 / MP-11: prove execution in the resumed workspace.
export function completedWorkspaceTool(rows, promptId, workspace) {
  if (!promptId) return false;
  return rows.some(row => {
    if (row.role !== 'tool' || row.promptId !== promptId || row.hidden
      || row.blobCollapsed === true || row.historyBlobLoading || row.historyBlobError) return false;
    const text = row.text ?? '';
    if (!/^\*\*[^\n]+\*\* · COMPLETED(?:\n|$)/.test(text)
      || !/^exit_code: 0$/m.test(text)) return false;
    const output = text.match(/\*\*Output\*\*\n`{3}[^\n]*\n([\s\S]*?)\n`{3}(?:\n|$)/)?.[1];
    return output?.split('\n').some(line => line.trim() === workspace) ?? false;
  });
}

export function completedTurnPromptId(rows, prompt, previousRows) {
  // Hydrating history can reuse row numbers. Prompt identities remain stable.
  const previousPrompts = new Set(previousRows.map(row => row.promptId));
  const promptId = rows.findLast(row => row.role === 'user'
    && row.text?.trim() === prompt && row.promptId
    && !previousPrompts.has(row.promptId))?.promptId;
  return promptId && rows.some(row => row.promptId === promptId
    && ['assistant', 'error'].includes(row.role) && row.text?.trim()) ? promptId : undefined;
}

export function completedHomeMcpTool(rows, promptId, marker, homeKernelPid, targetKernelPid) {
  if (!promptId || !Number.isSafeInteger(homeKernelPid) || homeKernelPid <= 1
    || !Number.isSafeInteger(targetKernelPid) || targetKernelPid <= 1
    || homeKernelPid === targetKernelPid) return false;
  return rows.some(row => {
    if (row.role !== 'tool' || row.promptId !== promptId || row.hidden
      || row.blobCollapsed === true || row.historyBlobLoading || row.historyBlobError) return false;
    const text = row.text ?? '';
    if (!/^\*\*Called [^\n]*mk04[_ ]ping\*\* · COMPLETED(?:\n|$)/.test(text)) return false;
    const output = text.match(/\*\*Output\*\*\n`{3}[^\n]*\n([\s\S]*?)\n`{3}(?:\n|$)/)?.[1];
    try {
      const result = JSON.parse(output);
      return !result.isError && result.content?.some(block => {
        if (block.type !== 'text') return false;
        const proof = JSON.parse(block.text);
        return proof.marker === marker && Array.isArray(proof.executionAncestry)
          && proof.executionAncestry.every(pid => Number.isSafeInteger(pid) && pid > 1)
          && proof.executionAncestry.includes(homeKernelPid)
          && !proof.executionAncestry.includes(targetKernelPid);
      }) === true;
    } catch { return false; }
  });
}
