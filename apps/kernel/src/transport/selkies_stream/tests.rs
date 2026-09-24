use super::*;
use crate::transport::relay_crypto;
use crate::transport::secure_display::{DisplayMessage, DisplayMessageKind, DisplayPeer};
use std::process::Stdio;
use tokio::time::{timeout, Duration};
use wait_timeout::ChildExt;

mod relay;

struct Desktop {
    context: String,
    name: String,
}

impl Desktop {
    fn docker(&self) -> Command {
        let mut command = Command::new("docker");
        command
            .args(["--context", &self.context])
            .kill_on_drop(true);
        command
    }

    async fn checked(&self, arguments: &[&str]) -> Vec<u8> {
        let output = timeout(
            Duration::from_secs(30),
            self.docker().args(arguments).output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "docker drill command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    async fn start() -> Self {
        let desktop = Self {
            context: std::env::var("CHARIOX_SELKIES_TEST_DOCKER_CONTEXT")
                .unwrap_or_else(|_| "default".to_owned()),
            name: format!(
                "chariox-selkies-encrypted-{}-{:x}",
                std::process::id(),
                rand::random::<u64>()
            ),
        };
        let image = std::env::var("CHARIOX_SELKIES_TEST_IMAGE")
            .expect("explicit packaged test image required");
        desktop.checked(&[
            "run", "--rm", "--init", "--detach", "--name", &desktop.name,
            "--user", "1000:1000", "--network", "none", "--cpus", "1",
            "--memory", "768m", "--memory-swap", "768m", "--pids-limit", "128",
            "--shm-size", "128m", "--env", "DISPLAY=:93", "--env", "OMP_NUM_THREADS=1",
            "--env", "HOME=/tmp/chariox-encrypted-test", "--env", "XDG_RUNTIME_DIR=/tmp/chariox-encrypted-test",
            &image, "sh", "-c", "mkdir -p /tmp/chariox-encrypted-test && exec Xvfb :93 -screen 0 640x480x24 -nolisten tcp -ac",
        ]).await;
        for _ in 0..30 {
            let output = desktop
                .docker()
                .args(["exec", &desktop.name, "xdpyinfo"])
                .output()
                .await
                .unwrap();
            if output.status.success() {
                desktop
                    .checked(&[
                        "exec",
                        &desktop.name,
                        "/opt/chariox-selkies/bin/python",
                        "/opt/chariox-slice/slice-selkies.py",
                        "start",
                    ])
                    .await;
                return desktop;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("Xvfb did not become ready");
    }

    fn stream(&self) -> Command {
        let mut command = self.docker();
        command.args([
            "exec",
            "-i",
            &self.name,
            "/opt/chariox-selkies/bin/python",
            "/opt/chariox-slice/slice-selkies-stream.py",
        ]);
        command
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        // Also runs on assertion failure. This uniquely named container belongs
        // to this drill; never prune images or another development container.
        let mut child = std::process::Command::new("docker")
            .args(["--context", &self.context, "rm", "--force", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let status = child.wait_timeout(Duration::from_secs(15)).unwrap();
        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("drill container cleanup timed out");
        }
        if !std::thread::panicking() {
            assert!(status.unwrap().success(), "drill container cleanup failed");
        }
    }
}

fn ciphers(stream_id: &str) -> (SecureDisplayChannel, SecureDisplayChannel) {
    let kernel_key = relay_crypto::generate_private_key_base64();
    let viewer_key = relay_crypto::generate_private_key_base64();
    let kernel_public = relay_crypto::public_key_from_private_key_base64(&kernel_key).unwrap();
    let viewer_public = relay_crypto::public_key_from_private_key_base64(&viewer_key).unwrap();
    (
        SecureDisplayChannel::new(kernel_key, viewer_public, stream_id, DisplayPeer::Kernel)
            .unwrap(),
        SecureDisplayChannel::new(viewer_key, kernel_public, stream_id, DisplayPeer::Viewer)
            .unwrap(),
    )
}

async fn receive(
    outgoing: &mut mpsc::Receiver<EncryptedRelayPayload>,
    viewer: &mut SecureDisplayChannel,
) -> DisplayMessage {
    timeout(Duration::from_secs(15), async {
        loop {
            let packet = outgoing
                .recv()
                .await
                .expect("encrypted stream ended before video");
            assert!(!serde_json::to_string(&packet)
                .unwrap()
                .contains("VIDEO_STARTED"));
            if let Some(message) = viewer.decode(&packet).unwrap() {
                return message;
            }
        }
    })
    .await
    .expect("live frame deadline exceeded")
}

#[tokio::test]
#[ignore = "requires an explicit local packaged Selkies image; one bounded Docker desktop"]
async fn real_selkies_video_crosses_encrypted_kernel_stream_and_denies_viewer_input() {
    let desktop = Desktop::start().await;
    let (kernel_cipher, mut viewer) = ciphers("live-private-stream");
    let (incoming_tx, incoming) = mpsc::channel(4);
    let (outgoing, outgoing_rx) = mpsc::channel(4);
    let (lease_tx, lease) = watch::channel(Some(Instant::now() + Duration::from_secs(30)));
    let task = tokio::spawn(forward_selkies_stream(
        desktop.stream(),
        kernel_cipher,
        incoming,
        outgoing,
        lease,
    ));
    let mut relay = relay::LiveRelay::connect(incoming_tx, outgoing_rx).await;
    let frame = loop {
        let message = receive(&mut relay.output, &mut viewer).await;
        if message.kind == DisplayMessageKind::Binary {
            break message.data;
        }
    };
    assert!(frame.len() > 10);
    assert_eq!(frame[0], 4);
    assert_eq!(u16::from_be_bytes([frame[6], frame[7]]), 640);
    assert_eq!(u16::from_be_bytes([frame[8], frame[9]]), 480);
    assert!(frame[10..].starts_with(&[0, 0, 0, 1]) || frame[10..].starts_with(&[0, 0, 1]));
    for packet in viewer
        .encode(DisplayMessageKind::Text, b"STOP_VIDEO")
        .unwrap()
    {
        relay.input.send(packet).await.unwrap();
    }
    loop {
        let message = receive(&mut relay.output, &mut viewer).await;
        if message.data == b"VIDEO_STOPPED" {
            break;
        }
    }
    for packet in viewer.encode(DisplayMessageKind::Text, b"kd,65").unwrap() {
        relay.input.send(packet).await.unwrap();
    }
    assert!(timeout(Duration::from_secs(8), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    drop(lease_tx);
    relay.close().await;
    desktop
        .checked(&[
            "exec",
            &desktop.name,
            "sh",
            "-c",
            "! ps -eo args | grep '[p]ython /opt/chariox-slice/slice-selkies-stream.py'",
        ])
        .await;
}

#[tokio::test]
#[ignore = "requires an explicit local packaged Selkies image; one bounded Docker desktop"]
async fn real_selkies_stream_obeys_kernel_lease_expiry_renewal_and_revocation() {
    let desktop = Desktop::start().await;
    for mode in ["expiry", "revocation", "renewal"] {
        let (kernel_cipher, mut viewer) = ciphers(mode);
        let (incoming_tx, incoming) = mpsc::channel(4);
        let (outgoing, mut outgoing_rx) = mpsc::channel(4);
        let (lease_tx, lease) = watch::channel(Some(Instant::now() + Duration::from_secs(30)));
        let task = tokio::spawn(forward_selkies_stream(
            desktop.stream(),
            kernel_cipher,
            incoming,
            outgoing,
            lease,
        ));
        while receive(&mut outgoing_rx, &mut viewer).await.kind != DisplayMessageKind::Binary {}
        match mode {
            "expiry" => {
                lease_tx
                    .send(Some(Instant::now() + Duration::from_millis(100)))
                    .unwrap();
            }
            "revocation" => {
                lease_tx.send(None).unwrap();
            }
            "renewal" => {
                lease_tx
                    .send(Some(Instant::now() + Duration::from_millis(400)))
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(150)).await;
                lease_tx
                    .send(Some(Instant::now() + Duration::from_secs(5)))
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(350)).await;
                for packet in viewer
                    .encode(DisplayMessageKind::Text, b"STOP_VIDEO")
                    .unwrap()
                {
                    incoming_tx.send(packet).await.unwrap();
                }
                while receive(&mut outgoing_rx, &mut viewer).await.data != b"VIDEO_STOPPED" {}
                lease_tx.send(None).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(timeout(Duration::from_secs(8), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        desktop
            .checked(&[
                "exec",
                &desktop.name,
                "sh",
                "-c",
                "! ps -eo args | grep '[p]ython /opt/chariox-slice/slice-selkies-stream.py'",
            ])
            .await;
    }
}
