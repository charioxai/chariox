import { chmod, mkdir, mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { workflowPublicationPackageDigest } from "@arroba/kernel-client/workflow-publication-package-digest"

export async function deployedWorkflowPackageFixture(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "arroba-deployed-release-package-"))
  await mkdir(join(root, "public"))
  const publication = Buffer.from(JSON.stringify({
    schema_version: 1,
    package_version: 3,
    publication_id: "publication-1",
    alias: "Demo app",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
    hooks: [{ id: "hook-1", endpoint_id: "endpoint-1", transport: "human_http" }],
  }, null, 2) + "\n")
  const launcher = Buffer.from("#!/bin/sh\nexit 0\n")
  const index = Buffer.from("<!doctype html><title>Demo</title>\n")
  const packageId = workflowPublicationPackageDigest([
    { path: "publication.json", content: publication, executable: false },
    { path: "public/index.html", content: index, executable: false },
    { path: "run.sh", content: launcher, executable: true },
  ])
  const contract = Buffer.from(JSON.stringify(deploymentContractFixture(packageId), null, 2) + "\n")
  await writeFile(join(root, "publication.json"), publication)
  await writeFile(join(root, "deployment-contract.json"), contract)
  await writeFile(join(root, "public/index.html"), index)
  await writeFile(join(root, "run.sh"), launcher)
  await chmod(join(root, "run.sh"), 0o755)
  return root
}

export function deploymentContractFixture(packageId: string) {
  return {
    schema_version: 1,
    package_id: packageId,
    artifact: {
      content_digest: packageId,
      digest_algorithm: "sha256",
      digest_scope: "package_files_excluding_deployment_contract",
    },
    source: {
      publication_id: "publication-1",
      session_id: "session-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      creator_user_id: "user-1",
      captured_at_ms: null,
    },
    compatibility: {
      package_version: 3,
      minimum_kernel_version: "0.1.0",
      minimum_local_daemon_protocol_version: 1,
    },
    routes: [{ id: "hook-1", path: "/prompt/*", required_roles: ["public"] }],
    provider_requirements: [],
    credential_slots: [],
    configuration: [],
    capabilities: {},
    resources: {},
    presentation: { kind: "agent_app", display_name: "Demo app" },
    signatures: [],
  }
}
