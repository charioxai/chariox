use super::*;

pub fn create_opencode_fixture_script(delay_seconds: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "chariox-opencode-fixture-{}-{}.sh",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be monotonic enough")
            .as_nanos()
    ));
    fs::write(&path, fixture_script_contents(delay_seconds))
        .expect("fixture script should be created");
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(&path)
            .expect("fixture script should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("fixture script should be executable");
    }
    path
}

fn fixture_script_contents(delay_seconds: u64) -> String {
    format!(
        r#"#!/bin/sh
PORT=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --port)
      PORT="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [ -z "$PORT" ] || [ -z "$CHARIOX_OPENCODE_PORT" ]; then
  exit 2
fi

export CHARIOX_OPENCODE_FIXTURE_LISTEN_PORT="$PORT"
export CHARIOX_OPENCODE_FIXTURE_MAX_SECONDS="{delay_seconds}"
python3 - <<'PY'
import os
import signal
import socket
import sys
import threading
import time

listen_port = int(os.environ["CHARIOX_OPENCODE_FIXTURE_LISTEN_PORT"])
target_port = int(os.environ["CHARIOX_OPENCODE_PORT"])
max_seconds = float(os.environ["CHARIOX_OPENCODE_FIXTURE_MAX_SECONDS"])
deadline = time.monotonic() + max_seconds
stopping = threading.Event()

def stop(_signum=None, _frame=None):
    stopping.set()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)

def relay(source, destination):
    try:
        while not stopping.is_set():
            chunk = source.recv(65536)
            if not chunk:
                break
            destination.sendall(chunk)
    except OSError:
        pass
    finally:
        for sock in (source, destination):
            try:
                sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                sock.close()
            except OSError:
                pass

def handle(client):
    try:
        upstream = socket.create_connection(("127.0.0.1", target_port), timeout=10)
    except OSError:
        client.close()
        return
    threading.Thread(target=relay, args=(client, upstream), daemon=True).start()
    threading.Thread(target=relay, args=(upstream, client), daemon=True).start()

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen_port))
    server.listen()
    server.settimeout(0.1)
    while not stopping.is_set() and time.monotonic() < deadline:
        try:
            client, _addr = server.accept()
        except socket.timeout:
            continue
        except OSError:
            break
        threading.Thread(target=handle, args=(client,), daemon=True).start()

sys.exit(0)
PY
"#
    )
}

pub fn output_timeout_ms() -> u64 {
    env::var("CHARIOX_HARNESS_OUTPUT_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(2_000)
}
