import assert from 'node:assert/strict';
import test from 'node:test';
import {completedWorkspaceTool} from './workspace-proof.mjs';

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
