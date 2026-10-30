import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import test from 'node:test';
import {completedWorkspaceTool, completedTurnPromptId, completedHomeMcpTool} from './workspace-proof.mjs';

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

test('MP-08/09/10/11 home MCP proof uses actual kernel ancestry and a completed current tool result', () => {
  const render = proof => '**Called chariox_mcp_mk04-proof.mk04 ping** · COMPLETED\n\n**Output**\n```json\n'
    + JSON.stringify({content: [{type: 'text', text: JSON.stringify(proof)}]}) + '\n```';
  const proof = {marker: 'fresh-marker', executionAncestry: [2345, 3456, 1234, 4567]};
  const row = {role: 'tool', promptId: 'current', text: render(proof)};
  assert.equal(completedHomeMcpTool([row], 'current', 'fresh-marker', 1234, 5678), true);
  for (const changed of [
    {...row, promptId: 'old'}, {...row, role: 'assistant'}, {...row, hidden: true},
    {...row, blobCollapsed: true}, {...row, historyBlobLoading: true}, {...row, historyBlobError: 'failed'},
    {...row, text: render({...proof, executionAncestry: [2345, 5678, 4567]})},
    {...row, text: render({...proof, executionAncestry: [2345, 1234, 5678]})},
    {...row, text: render({...proof, executionAncestry: [2345, 0, 1234]})},
    {...row, text: render({...proof, marker: 'old-marker'})},
    {...row, text: render({marker: proof.marker, executionHome: '/label-only'})},
    {...row, text: row.text.replace('COMPLETED', 'RUNNING')},
    {...row, text: row.text.replace('COMPLETED', 'ERROR')},
    {...row, text: row.text.replace('```json\n', '```json\ninvalid')}
  ]) assert.equal(completedHomeMcpTool([changed], 'current', 'fresh-marker', 1234, 5678), false);
  for (const pid of [0, 1, -1, NaN, undefined]) assert.equal(completedHomeMcpTool([row], 'current', 'fresh-marker', pid, 5678), false);
});

test('MP-08/09/10/11 actual proof server reports its live process ancestry without control environment', () => {
  const driver = readFileSync(new URL('./composed-move.mjs', import.meta.url), 'utf8');
  const start = driver.indexOf("`import readline");
  const end = driver.indexOf('`);', start);
  assert.ok(start >= 0 && end > start);
  const code = vm.runInNewContext(driver.slice(start, end + 1));
  const env = {...process.env};
  delete env.CHARIOX_HOME;
  const run = spawnSync(process.execPath, ['--input-type=module', '-e', code], {
    env, input: JSON.stringify({jsonrpc: '2.0', id: 1, method: 'tools/call', params: {arguments: {marker: 'stdio-proof'}}}) + '\n',
    encoding: 'utf8', timeout: 5000, maxBuffer: 65536,
  });
  assert.equal(run.status, 0, run.stderr);
  const result = JSON.parse(run.stdout).result;
  const proof = JSON.parse(result.content[0].text);
  assert.equal(proof.marker, 'stdio-proof');
  assert.ok(proof.executionAncestry.includes(process.pid), 'actual parent must appear behind the provider child');
  const row = {role: 'tool', promptId: 'current', text: '**Called mk04_ping** · COMPLETED\n\n**Output**\n```json\n' + JSON.stringify(result) + '\n```'};
  assert.equal(completedHomeMcpTool([row], 'current', 'stdio-proof', process.pid, 999999999), true);
});
