//! A kernel at its descriptor limit refuses the terminal it cannot open, in
//! plain words, and the terminals it already runs keep running.
#![cfg(unix)]

use std::time::Duration;

use base64::Engine;
use serde_json::json;

#[path = "support/real_kernel.rs"]
mod real_kernel;
use real_kernel::{connect, create_session, terminal};

#[test]
fn a_kernel_out_of_descriptors_refuses_the_next_terminal_and_keeps_the_others() {
    let scratch = std::env::temp_dir().join(format!("odl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let kernel = real_kernel::start(&scratch, &[], Some(320));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut socket = connect(&kernel).await;
            let session_id = create_session(&mut socket, &scratch).await;
            let witness = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "open", "cols": 80, "rows": 24, "command": ["/bin/sh"]}),
            )
            .await
            .unwrap()["terminal"]["terminal_id"]
                .as_str()
                .unwrap()
                .to_string();

            // A Room holds 16 terminals: fill Rooms until the kernel runs short.
            let mut refused = None;
            let mut room = session_id.clone();
            for opened_so_far in 0..200 {
                if opened_so_far % 8 == 0 {
                    room = create_session(&mut socket, &scratch).await;
                }
                let opened = terminal(
                    &mut socket,
                    &room,
                    json!({"operation": "open", "cols": 80, "rows": 24,
                        "command": ["/bin/sh", "-c", "sleep 600"]}),
                )
                .await;
                if let Err(error) = opened {
                    refused = Some(error);
                    break;
                }
            }
            // The sampler ticks several times while descriptors stay short.
            tokio::time::sleep(Duration::from_secs(4)).await;
            let before = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "read_output", "terminal_id": witness,
                    "after_offset": 0, "wait_ms": 0}),
            )
            .await
            .unwrap()["output"]["terminal"]["output_offset"]
                .as_u64()
                .unwrap();
            let typed = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "input", "terminal_id": witness,
                    "data_base64": base64::engine::general_purpose::STANDARD
                        .encode("echo still-$((40+2))\n")}),
            )
            .await
            .unwrap_or_else(|error| panic!("the first terminal ended: {error}"));
            assert_eq!(typed["input"]["accepted"], true, "{typed}");
            let waited = terminal(
                &mut socket,
                &session_id,
                json!({"operation": "wait", "terminal_id": witness,
                    "watches": [{"kind": "output_contains", "text": "still-42"}],
                    "after_offset": before, "timeout_ms": 20000}),
            )
            .await
            .unwrap();
            assert_eq!(
                waited["waited"]["fired"]["kind"], "output_matched",
                "{waited}"
            );

            let refused = refused.expect("the kernel never reached its descriptor limit");
            assert_eq!(refused["code"], "too_many_open_files", "{refused}");
            assert_eq!(
                refused["message"],
                "too many open files in this kernel; close a terminal and try again",
                "{refused}"
            );
        });
}
