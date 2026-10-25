// Points a CLI cloud client at its own disposable home (ADMISSION_DRILL_CLI_HOME) before the CLI modules load.
const dir = process.env.ADMISSION_DRILL_CLI_HOME
if (!dir) throw new Error('set ADMISSION_DRILL_CLI_HOME to a scratch directory for the client login')
Object.assign(process.env, { HOME: `${dir}/user`, CHARIOX_HOME: `${dir}/kernel`, XDG_CONFIG_HOME: `${dir}/config`, XDG_STATE_HOME: `${dir}/state`, XDG_DATA_HOME: `${dir}/data`, XDG_CACHE_HOME: `${dir}/cache`, TMPDIR: `${dir}/tmp` })
for (const k of Object.keys(process.env)) if (k.startsWith('CHARIOX_') && k !== 'CHARIOX_HOME') delete process.env[k]
