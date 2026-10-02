import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const here = path.dirname(fileURLToPath(import.meta.url))
const root = process.env.HIDDENINPUT_BUILD_ROOT ?? path.resolve(here, "../../..")
const python = String.raw`
import errno, fcntl, json, os, pathlib, pty, select, signal, struct, subprocess, tempfile, termios, time

root, fixture, bun, surface, command = __import__('sys').argv[1:]
secret = 'hidden-synthetic-' + os.urandom(8).hex() + ('-pass' if os.environ.get('HIDDENINPUT_PLAIN') == '1' else '-päss')
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 100, 0, 0))
output = bytearray()
proc = None
state_root = os.environ.get('HIDDENINPUT_STATE_ROOT', os.path.expanduser('~/.chariox/dev/hiddeninput-pty'))
pathlib.Path(state_root).mkdir(parents=True, exist_ok=True, mode=0o700)
with tempfile.TemporaryDirectory(prefix='hiddeninput-pty-', dir=state_root) as state:
    env = {**os.environ, 'TERM': 'xterm-256color', 'CHARIOX_HOME': state, 'CHARIOX_LOG_DIR': state + '/logs',
           'HIDDENINPUT_BUILD_ROOT': root, 'HIDDENINPUT_FIXTURE_VALUE': secret}
    def read_until(marker, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if marker in output: return
            if b'HIDDEN_FAIL' in output:
                safe = __import__('re').search(rb'FIXTURE_REASON:([^\r\n]+)', output)
                raise RuntimeError(safe.group(1).decode() if safe else 'fixture failed')
            if select.select([master], [], [], .05)[0]:
                try: output.extend(os.read(master, 65536))
                except OSError as error:
                    if error.errno == errno.EIO: break
                    raise
        if secret.encode() in output or secret[:-5].encode() in output:
            raise RuntimeError('typed secret observed in PTY output')
        raise RuntimeError('PTY timeout (no secret observed), bytes=' + str(len(output)) + ', failure=' + str(b'HIDDEN_FAIL' in output))
    try:
        proc = subprocess.Popen([bun if surface == 'tui' else 'node', fixture, surface, command],
             stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
        if surface == 'shell':
            read_until(b'@ ')
            if command == 'run':
                os.write(master, b'provider setup-token claude --run\r')
                read_until(b'fixture-login')
                os.write(master, b'provider login-input fixture-login\r')
                read_until(b'login input: ')
            else:
                text = {'credential':'credential set synthetic', 'cancel':'credential set synthetic', 'setup':'provider setup-token claude'}[command]
                os.write(master, text.encode() + b'\r')
                read_until(b'credential synthetic: ' if command in ['credential', 'cancel'] else b'Claude setup token for default: ')
        else: read_until(b'HIDDEN_READY')
        # Fragment UTF-8 and paste delimiters. Newline in paste must not submit.
        data = secret.encode() if os.environ.get('HIDDENINPUT_PLAIN') == '1' else b'\x1b[200~' + secret.encode() + b'\x1b[201~'
        for byte in data:
            os.write(master, bytes([byte]))
            time.sleep(.002)
        time.sleep(.2)
        os.write(master, b'\x03' if command == 'cancel' else b'\r')
        if surface == 'shell' and command != 'cancel':
            time.sleep(.2)
            os.write(master, b'exit\r')
        read_until(b'HIDDEN_PASS')
        code = proc.wait(timeout=5)
        # ECHO was on before launch and must be restored after the TUI/shell exits.
        echo_restored = bool(termios.tcgetattr(slave)[3] & termios.ECHO)
        leaked = secret.encode() in output or secret[:-5].encode() in output
        if leaked: raise RuntimeError('typed secret observed in PTY output')
        if code != 0 or not echo_restored: raise RuntimeError('PTY restoration or exit assertion failed')
        print(json.dumps({'surface':surface, 'command':command, 'no_echo':True, 'echo_restored':True, 'exit':code}))
    finally:
        if proc is not None and proc.poll() is None:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
        os.close(master)
        os.close(slave)
`

for (const [surface, command] of [
  ["shell", "credential"], ["shell", "setup"], ["shell", "cancel"],
  ["tui", "credential"], ["tui", "setup"], ["tui", "login"], ["tui", "embedded"], ["tui", "cancel"],
  ...(process.env.HIDDENINPUT_SETUP_RUN === "1" ? [["shell", "run"], ["tui", "run"]] : []),
]) {
  test(`MP-08/MP-10/MP-11 ${surface} ${command}: secret absent from PTY output`, () => {
    const result = spawnSync("python3", ["-c", python, root,
      path.join(root, "apps/cli/scripts/lib/hidden-input-pty-fixture.mjs"),
      process.env.HIDDENINPUT_BUN ?? "bun", surface, command], { encoding: "utf8", timeout: 30000 })
    assert.equal(result.status, 0, result.stderr)
    const proof = JSON.parse(result.stdout)
    assert.equal(proof.no_echo, true)
    assert.equal(proof.echo_restored, true)
  })
}
