import assert from "node:assert/strict";

export function drillEnvironment(env = process.env) {
  assert.equal(process.platform, "linux");
  if (env.CHARIOX_CHROMIUM_DRILL_ENVIRONMENT === "builder") {
    assert.ok(!env.GITHUB_ACTIONS && !env.RUNNER_ENVIRONMENT && !env.GITHUB_REPOSITORY,
      "builder evidence must not claim a GitHub runner identity");
    return "builder";
  }
  assert.ok(!env.CHARIOX_CHROMIUM_DRILL_ENVIRONMENT, "unknown drill environment");
  assert.equal(env.GITHUB_ACTIONS, "true", "select the explicitly labelled builder environment outside GitHub");
  assert.equal(env.RUNNER_ENVIRONMENT, "github-hosted");
  assert.equal(env.GITHUB_REPOSITORY, "charioxai/chariox");
  return "github-hosted";
}
