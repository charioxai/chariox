import process from "node:process"
import { runContextCommand } from "./context-command.js"
import { runCloudClientCommand } from "./cloud-client-command.js"
import { runSudoCommand } from "./sudo-command.js"
import { runAccessCommand } from "./access-command.js"

import { render } from "@opentui/solid"

import { CharioxCliApp } from "./cli-app-root.js"
import { redactCliStartupArgs } from "./cli-process-logging.js"
import { bootstrapCliRuntime } from "./cli-runtime-bootstrap.js"
import {
  OPEN_CONSOLE_ON_ERROR,
  formatError,
  getLogger,
  processLoggers,
  transcriptParserRegistration,
} from "./cli-runtime-singletons.js"
import { runLogViewer } from "./logs.js"
import { runClaudeNativeTui } from "./native-tui/claude.js"
import { runCodexNativeTui } from "./native-tui/codex.js"
import { runOpenCodeNativeTui } from "./native-tui/opencode.js"
import { runPublicationDeploymentCommand } from "./publication-deployment-command.js"
import { runDeployedWorkflowCommand } from "./deployed-workflow-command.js"
import { runAppCommand } from "./app-command.js"
import { assertSolidClientBuild } from "./solid-client-build.js"

async function main() {
  assertSolidClientBuild()
  const argv = process.argv.slice(2)
  if (await runContextCommand(argv)) return
  if (await runCloudClientCommand(argv)) return
  if (await runSudoCommand(argv)) return
  if (await runAccessCommand(argv)) return
  if (await runAppCommand(argv)) return
  if (argv[0] === "logs") {
    await runLogViewer(argv.slice(1))
    return
  }
  if (argv[0] === "opencode") {
    await runOpenCodeNativeTui(argv.slice(1))
    return
  }
  if (argv[0] === "claude") {
    await runClaudeNativeTui(argv.slice(1))
    return
  }
  if (argv[0] === "codex") {
    await runCodexNativeTui(argv.slice(1))
    return
  }
  if (await runPublicationDeploymentCommand(argv)) {
    return
  }
  if (await runDeployedWorkflowCommand(argv)) {
    return
  }

  transcriptParserRegistration.ensureRegistered()
  processLoggers.initialize("cli")
  getLogger("cli.main")?.info("starting cli process", { argv: redactCliStartupArgs(argv) })
  const runtimeBootstrap = await bootstrapCliRuntime({
    argv,
    cwd: process.cwd(),
    logger: getLogger("cli.main"),
  })
  if (runtimeBootstrap.kind === "deleted_session") {
    return
  }
  await render(
    () => <CharioxCliApp bootstrap={runtimeBootstrap.bootstrap} />,
    {
      targetFps: 60,
      gatherStats: false,
      exitOnCtrlC: false,
      useKittyKeyboard: {},
      useMouse: true,
      enableMouseMovement: false,
      useAlternateScreen: true,
      autoFocus: true,
      openConsoleOnError: OPEN_CONSOLE_ON_ERROR,
    },
  )
  getLogger("cli.main")?.info("render mounted")
}

void main().catch((error) => {
  getLogger("cli.main")?.error("cli process failed", {
    error: formatError(error),
  })
  process.stderr.write(`${formatError(error)}\n`)
  process.exit(1)
})
