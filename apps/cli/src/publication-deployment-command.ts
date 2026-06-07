import {
  changePublicationDeployment,
  createPublicationDeploymentFromPackage,
  getPublicationDeployment,
  listPublicationDeploymentLogs,
  listPublicationDeployments,
  type PublicationDeploymentMode,
} from "./publication-deployment-api.js"
import {
  loadPreferences,
  relayCloudProfile,
} from "./preferences.js"

export async function runPublicationDeploymentCommand(argv: readonly string[]): Promise<boolean> {
  if (argv[0] !== "publication") return false
  const profile = relayCloudProfile(await loadPreferences())
  if (!profile) {
    throw new Error("cloud is not linked. Run /cloud login from the TUI before deploying publications.")
  }
  const command = argv[1]
  if (command === "deploy") {
    const packagePath = argv[2]
    if (!packagePath) throw new Error("usage: arroba publication deploy <package-dir|publication.json> --mode local-runtime|hosted-container")
    const options = parseDeployOptions(argv.slice(3))
    const deployment = await createPublicationDeploymentFromPackage({
      profile,
      packagePath,
      mode: options.mode,
      ...(options.slug !== undefined ? { slug: options.slug } : {}),
      ...(options.credentialProfile !== undefined ? { credentialProfile: options.credentialProfile } : {}),
      ...(options.start !== undefined ? { start: options.start } : {}),
    })
    process.stdout.write([
      `deployment ${deployment.id}`,
      `mode ${deployment.mode}`,
      `status ${deployment.status}`,
      `url ${deployment.publicBaseUrl}`,
      options.mode === "local_runtime" ? `serve arroba serve ${packagePath} <port> --cloud-deployment ${deployment.id}` : null,
    ].filter(Boolean).join("\n") + "\n")
    return true
  }
  if (command === "deployments") {
    await runDeploymentsCommand(profile, argv.slice(2))
    return true
  }
  throw new Error("usage: arroba publication deploy|deployments")
}

async function runDeploymentsCommand(
  profile: NonNullable<ReturnType<typeof relayCloudProfile>>,
  argv: readonly string[],
): Promise<void> {
  const command = argv[0] ?? "list"
  if (command === "list") {
    const deployments = await listPublicationDeployments(profile)
    for (const deployment of deployments) {
      process.stdout.write(`${deployment.id}\t${deployment.mode}\t${deployment.status}\t${deployment.publicBaseUrl}\n`)
    }
    return
  }
  const deploymentId = argv[1]
  if (!deploymentId) throw new Error("usage: arroba publication deployments show|logs|stop|restart <deployment-id>")
  if (command === "show") {
    process.stdout.write(JSON.stringify(await getPublicationDeployment(profile, deploymentId), null, 2) + "\n")
    return
  }
  if (command === "logs") {
    const logs = await listPublicationDeploymentLogs(profile, deploymentId)
    for (const entry of logs) {
      process.stdout.write(`${entry.occurredAt}\t${entry.level}\t${entry.message}\n`)
    }
    return
  }
  if (command === "stop" || command === "restart") {
    await changePublicationDeployment(profile, deploymentId, command)
    process.stdout.write(`${command} requested for ${deploymentId}\n`)
    return
  }
  throw new Error("usage: arroba publication deployments list|show|logs|stop|restart")
}

function parseDeployOptions(argv: readonly string[]): {
  readonly mode: PublicationDeploymentMode
  readonly slug?: string
  readonly credentialProfile?: string
  readonly start?: boolean
} {
  let mode: PublicationDeploymentMode | undefined
  let slug: string | undefined
  let credentialProfile: string | undefined
  let start = false
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--mode") {
      const value = argv[++index]
      if (value === "local-runtime" || value === "local_runtime") mode = "local_runtime"
      else if (value === "hosted-container" || value === "hosted_container") mode = "hosted_container"
      else throw new Error("--mode must be local-runtime or hosted-container")
    } else if (arg === "--slug") {
      slug = argv[++index]
    } else if (arg === "--credential-profile") {
      credentialProfile = argv[++index]
    } else if (arg === "--start") {
      start = true
    } else {
      throw new Error(`unknown publication deploy option ${arg}`)
    }
  }
  if (!mode) throw new Error("arroba publication deploy requires --mode local-runtime|hosted-container")
  return {
    mode,
    ...(slug !== undefined ? { slug } : {}),
    ...(credentialProfile !== undefined ? { credentialProfile } : {}),
    start,
  }
}
