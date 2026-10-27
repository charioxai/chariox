import assert from 'node:assert/strict';
import test from 'node:test';
import {completedWorkspaceTool, completedTurnPromptId} from './workspace-proof.mjs';

const workspace='/owned/target/workspace';
const text='**Ran pwd** · COMPLETED\n\n**Command**\n```bash\n$ pwd\n```\n\n**Output**\n```text\n'+workspace+'\n```\n\n**Details**\n```text\nexit_code: 0\n```';
const tool={id:2,promptId:'fresh',role:'tool',text,blobCollapsed:false};
test('MP-08/09/10/11 workspace proof needs fresh completed shell output',()=>{
  assert.equal(completedWorkspaceTool([tool],'fresh',workspace),true);
  for(const row of [
    {...tool,role:'user'}, {...tool,role:'assistant'}, {...tool,role:'status'}, {...tool,role:'error'},
    {...tool,promptId:'old'}, {...tool,hidden:true}, {...tool,blobCollapsed:true},
    {...tool,historyBlobLoading:true}, {...tool,historyBlobError:'load failed'},
    {...tool,text:text.replace('COMPLETED','RUNNING')},
    {...tool,text:text.replace('COMPLETED','ERROR')},
    {...tool,text:text.replace('exit_code: 0','exit_code: 1')},
    {...tool,text:'**Ran pwd** · COMPLETED\n\n**Command**\n```bash\n$ cd '+workspace+'\n```'},
    {...tool,text:'',blobTitle:workspace,blobSummary:workspace},
    {...tool,text:text.replace(workspace,workspace+'-other')},
  ]) assert.equal(completedWorkspaceTool([row],'fresh',workspace),false,JSON.stringify(row));
  assert.equal(completedWorkspaceTool([tool],null,workspace),false);
});

test('MP-08/09/10/11 native turn completion survives TUI row renumbering', () => {
  const previous = [{id: 1, role: 'user', promptId: 'old', text: 'old'},
    {id: 2, role: 'assistant', promptId: 'old', text: 'old answer'}];
  const current = [{id: 1, role: 'user', promptId: 'current', text: 'Reply ACK'},
    {id: 2, role: 'assistant', promptId: 'current', text: 'ACK'}];
  assert.equal(completedTurnPromptId(current, 'Reply ACK', previous), 'current');
  for (const rows of [previous, current.slice(0, 1),
    [current[0], {...current[1], promptId: 'old'}],
    [current[0], {...current[1], text: ''}],
    [current[0], {...current[1], role: 'tool'}]]) {
    assert.equal(completedTurnPromptId(rows, 'Reply ACK', previous), undefined);
  }
  assert.equal(completedTurnPromptId(current, 'different prompt', previous), undefined);
  assert.equal(completedTurnPromptId(current, 'Reply ACK', current), undefined);
});
