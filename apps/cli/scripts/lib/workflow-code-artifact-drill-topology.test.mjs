import assert from 'node:assert/strict'
import { access, mkdtemp, readFile, readdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import ts from 'typescript'

import {
  EXAMPLE_RUNTIME_EXPECTATIONS,
  EXAMPLE_TOPOLOGY_EXPECTATIONS,
  workflowCodeExamples,
  writeWorkflowCodeArtifactSkillSource,
} from './workflow-code-artifact-drill-topology.mjs'
import { remoteWorkflowCodeRunnerSource } from './workflow-code-artifact-drill-remote-runner.mjs'
import { buildKernel } from './workflow-code-artifact-drill-runtime.mjs'

test('workflow-code artifact topology helpers keep their filesystem dependencies', async () => {
  const tempDir = await mkdtemp(path.join(os.tmpdir(), 'arroba-workflow-code-topology-'))
  try {
    const skillDir = await writeWorkflowCodeArtifactSkillSource(tempDir, 'workflow-code-test-skill')
    const skillSource = await readFile(path.join(skillDir, 'SKILL.md'), 'utf8')
    assert.match(skillSource, /name: workflow-code-test-skill/)

    const examples = await workflowCodeExamples()
    assert.ok(examples.some((example) => example.name === 'generate-filter.js'))
    assert.ok(examples.every((example) => example.source.length > 0))
    const exampleNames = examples.map((example) => example.name).sort()
    assert.deepEqual(Object.keys(EXAMPLE_TOPOLOGY_EXPECTATIONS).sort(), exampleNames)
    assert.deepEqual(Object.keys(EXAMPLE_RUNTIME_EXPECTATIONS).sort(), exampleNames)
  } finally {
    await rm(tempDir, { recursive: true, force: true })
  }
})

test('workflow-code artifact drill launches the kernel binary Cargo built', async () => {
  const kernelPath = buildKernel()
  await access(kernelPath)
  assert.match(kernelPath, /\/target\/debug\/arroba-kernel$/)
})

test('remote workflow-code runner covers the complete example runtime suite', () => {
  const source = remoteWorkflowCodeRunnerSource()

  assert.match(source, /'planner-worker-reviewer\.js': \{ completed: true \}/)
  assert.match(source, /'routing\.js': \{ specialist: 1 \}/)
  assert.doesNotMatch(source, /'routing\.js': \{ specialist: 'code' \}/)
})

test('workflow-code artifact drill modules have no unresolved runtime identifiers', async () => {
  const repoRoot = process.cwd()
  const libDir = path.join(repoRoot, 'apps', 'cli', 'scripts', 'lib')
  const modulePaths = (await readdir(libDir))
    .filter((name) => name.startsWith('workflow-code-artifact-drill-') && name.endsWith('.mjs') && !name.endsWith('.test.mjs'))
    .map((name) => path.join(libDir, name))
  modulePaths.push(path.join(repoRoot, 'apps', 'cli', 'scripts', 'workflow-code-artifact-drill.mjs'))

  const program = ts.createProgram(modulePaths, {
    allowJs: true,
    checkJs: true,
    module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext,
    noEmit: true,
    skipLibCheck: true,
    target: ts.ScriptTarget.ES2022,
  })
  const unresolved = ts.getPreEmitDiagnostics(program)
    .filter((diagnostic) => diagnostic.code === 2304 || diagnostic.code === 2552)
    .map((diagnostic) => {
      const location = diagnostic.file && diagnostic.start != null
        ? diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start)
        : null
      const prefix = diagnostic.file && location
        ? `${path.relative(repoRoot, diagnostic.file.fileName)}:${location.line + 1}:${location.character + 1}: `
        : ''
      return `${prefix}${ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n')}`
    })
  assert.deepEqual(unresolved, [])
})
