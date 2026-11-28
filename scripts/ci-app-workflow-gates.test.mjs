import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../.github/workflows/apps-components.yml', import.meta.url), 'utf8');

function jobs(source) {
  const axis = name => source.match(new RegExp(`^        ${name}: \\[([^\\]]+)\\]`, 'm'))?.[1].split(',').map(value => value.trim()) ?? [];
  const os = axis('os'), checks = axis('check');
  assert.deepEqual(checks, ['components', 'native-cli'], 'native CLI needs its own mandatory compile job');
  assert.deepEqual(os, ['ubuntu-latest', 'macos-15'], 'both supported platforms must run');
  assert.match(source, /^      max-parallel: 1$/m);
  assert.match(source, /^    timeout-minutes: 45$/m);
  const steps = source.split(/(?=^      - (?:name|uses):)/m).filter(block => block.startsWith('      - name:')).map(block => ({
    name: block.match(/^      - name: (.+)$/m)[1],
    condition: block.match(/^        if: (.+)$/m)?.[1],
    run: block.split(/^        run: /m)[1] ?? '',
  }));
  return os.flatMap(platform => checks.map(check => ({ platform, check, steps: steps.filter(step => {
    if (!step.condition) return true;
    return step.condition.split(' && ').map(condition => {
      if (condition === 'always()') return true;
      if (condition === "runner.os == 'Linux'") return platform === 'ubuntu-latest';
      const selected = condition.match(/^matrix\.check == '(components|native-cli)'$/);
      assert.ok(selected, `unverified App workflow condition: ${condition}`);
      return check === selected[1];
    }).every(Boolean);
  }) })));
}

function nativeJobs(source) {
  const expanded = jobs(source);
  const native = expanded.filter(job => job.check === 'native-cli');
  assert.equal(native.length, 2);
  for (const job of expanded) {
    const dispatch = job.steps.find(step => step.name === 'Test native CLI developer dispatch');
    if (job.check === 'components') {
      assert.equal(dispatch, undefined, 'native compilation must not use the remainder of the kernel job deadline');
    } else {
      assert.ok(dispatch, `${job.platform} must actually run native CLI assertions`);
      assert.match(dispatch.run, /cargo test --locked -p chariox-kernel --bin chariox-cli app_developer::tests::/);
      assert.equal(job.steps.some(step => step.name === 'Test kernel App routing and durable writer'), false);
    }
  }
}

function protocolJobs(source) {
  for (const job of jobs(source)) {
    const contract = job.steps.find(step => step.name === 'Check shared protocol snapshots and terminal catalog');
    if (job.check === 'native-cli') {
      assert.equal(contract, undefined);
    } else {
      assert.ok(job.steps.some(step => step.name === 'Test kernel App routing and durable writer'));
      assert.ok(contract, `${job.platform} must run protocol and catalog assertions`);
      assert.match(contract.run, /cargo test --locked -p chariox-kernel --lib protocol_shapes/);
      assert.match(contract.run, /cargo test --locked -p chariox-kernel --lib terminal_command_catalog::tests::/);
    }
  }
}

test('both platforms run native CLI assertions in a separate mandatory job', () => nativeJobs(workflow));
test('both platforms run protocol snapshots and terminal catalog in the components job', () => protocolJobs(workflow));
test('moving native dispatch to the components guard is detected', () => {
  const wrong = workflow.replace("name: Test native CLI developer dispatch\n        if: matrix.check == 'native-cli'", "name: Test native CLI developer dispatch\n        if: matrix.check == 'components'");
  assert.notEqual(wrong, workflow);
  assert.throws(() => nativeJobs(wrong));
});
test('moving protocol and catalog checks to the native guard is detected', () => {
  const wrong = workflow.replace("name: Check shared protocol snapshots and terminal catalog\n        if: matrix.check == 'components'", "name: Check shared protocol snapshots and terminal catalog\n        if: matrix.check == 'native-cli'");
  assert.notEqual(wrong, workflow);
  assert.throws(() => protocolJobs(wrong));
});
