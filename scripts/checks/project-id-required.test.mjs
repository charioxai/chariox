import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './project-id-required.mjs'

const SESSION = 'apps/kernel/src/session/runtime_session.rs'
const WIRE = 'apps/kernel/src/local/api/types/session_control.rs'
const clean = {
  [SESSION]: 'pub struct RuntimeSession {\n    project_id: ProjectId,\n}\n',
  [WIRE]: 'pub struct SessionCreated {\n    pub project_id: String,\n}\n',
}
const planted = (path, text, expected) => {
  const r = makeRepo(clean)
  assert.equal(r.run(check.name).status, 0, 'the clean tree passes')
  r.write(path, text)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`))
  r.dispose()
}

test('an optional ProjectId is red', () => {
  planted(SESSION, 'pub struct RuntimeSession {\n    project_id: Option<ProjectId>,\n}\n', /runtime_session\.rs:2: makes the (ProjectId|session project_id) optional/)
  planted('apps/kernel/src/app/other.rs', 'fn f(id: Option<crate::session::ProjectId>) {}\n', /other\.rs:1: makes the ProjectId optional/)
})

test('building an empty project id is red', () => {
  planted(SESSION, 'fn new() -> Self {\n    Self { project_id: String::new() }\n}\n', /runtime_session\.rs:2: builds an empty project id/)
})

test('assigning the project id after the Room was built is red', () => {
  planted(SESSION, 'impl RuntimeSession {\n    fn assign_project_id(&mut self) {}\n}\n', /runtime_session\.rs:2: assigns the project id after the Room was built/)
})

test('a defaulted or skippable project_id in the session or its wire type is red', () => {
  planted(SESSION, 'pub struct RuntimeSession {\n    #[serde(default)]\n    project_id: ProjectId,\n}\n', /runtime_session\.rs:2: lets a stored or sent session omit its project_id/)
  planted(WIRE, 'pub struct SessionCreated {\n    #[serde(default, skip_serializing_if = "String::is_empty")]\n    pub project_id: String,\n}\n', /session_control\.rs:2: lets a stored or sent session omit its project_id/)
})

test('an optional project filter outside the session types, a comment and a test module pass', () => {
  const r = makeRepo({
    ...clean,
    'apps/kernel/src/app/filter.rs': 'fn list(project_id: Option<&str>) {}\n// project_id: String::new()\n#[cfg(test)]\nmod tests {\n    fn t() { let _ = Session { project_id: String::new() }; }\n}\n',
  })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
