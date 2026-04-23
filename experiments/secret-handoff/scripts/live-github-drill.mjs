#!/usr/bin/env node

import { execFile } from "node:child_process"
import { promisify } from "node:util"

import { SecretHandoffHarness } from "../src/secret-harness.mjs"

const execFileAsync = promisify(execFile)

async function main() {
  const token = await loadGithubToken()
  if (!token.value) {
    console.log(JSON.stringify({
      ok: false,
      skipped: true,
      reason: "No GITHUB_TOKEN/GH_TOKEN env value and `gh auth token` did not return a token.",
    }, null, 2))
    return
  }

  const credentials = {
    github: token.source === "env"
      ? {
          source: "env",
          env: token.envName,
          allowed_hosts: ["api.github.com"],
          allowed_uses: ["http"],
          injection: {
            kind: "header",
            name: "authorization",
            value: "Bearer ${secret}",
          },
        }
      : {
          source: "vault",
          value: token.value,
          allowed_hosts: ["api.github.com"],
          allowed_uses: ["http"],
          injection: {
            kind: "header",
            name: "authorization",
            value: "Bearer ${secret}",
          },
        },
  }

  const runtime = new SecretHandoffHarness({
    kernelEnv: process.env,
    credentials,
  })
  const inspected = await runtime.launchFakeAgent({
    names: ["GITHUB_TOKEN", "GH_TOKEN", "PATH", "HOME"],
  })
  const githubTokenVisible = inspected.env.GITHUB_TOKEN !== null
  const ghTokenVisible = inspected.env.GH_TOKEN !== null
  if (githubTokenVisible || ghTokenVisible) {
    throw new Error("fake agent could read a GitHub token env value")
  }

  const result = await runtime.httpRequestWithCredential({
    credential_id: "github",
    url: "https://api.github.com/user",
    headers: {
      accept: "application/vnd.github+json",
      "user-agent": "arroba-secret-handoff-spike",
      "x-github-api-version": "2022-11-28",
    },
  })

  if (result.status !== 200) {
    console.log(JSON.stringify({
      ok: false,
      status: result.status,
      body: summarizeGitHubError(result.body),
      token_source: token.source,
      agent_env_scrubbed: true,
    }, null, 2))
    process.exitCode = 1
    return
  }

  const serialized = JSON.stringify(result)
  if (serialized.includes(token.value)) {
    throw new Error("runtime result unexpectedly included the GitHub token")
  }

  console.log(JSON.stringify({
    ok: true,
    service: "github",
    endpoint: "GET https://api.github.com/user",
    status: result.status,
    login: result.body?.login ?? null,
    token_source: token.source,
    agent_env_scrubbed: true,
    result_contains_token: false,
  }, null, 2))
}

async function loadGithubToken() {
  if (process.env.GITHUB_TOKEN) {
    return { source: "env", envName: "GITHUB_TOKEN", value: process.env.GITHUB_TOKEN }
  }
  if (process.env.GH_TOKEN) {
    return { source: "env", envName: "GH_TOKEN", value: process.env.GH_TOKEN }
  }
  try {
    const { stdout } = await execFileAsync("gh", ["auth", "token"], {
      env: process.env,
      timeout: 5000,
      maxBuffer: 1024 * 1024,
    })
    const value = stdout.trim()
    if (!value) return { source: null, value: null }
    return { source: "gh-auth-token", value }
  } catch {
    return { source: null, value: null }
  }
}

function summarizeGitHubError(body) {
  if (!body || typeof body !== "object") return body
  return {
    message: body.message,
    documentation_url: body.documentation_url,
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error))
  process.exit(1)
})
