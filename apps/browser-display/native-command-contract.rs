// Focused serde boundary for the production native command. The text aliases
// belong to unrelated input variants; no display DTO or serde policy is copied.
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
type RoomEnvironmentKeyboardInput = String;
type RoomEnvironmentClipboardText = String;
#[path = "../kernel/src/local/api/types/kernel_computer.rs"]
mod command;
fn main() {
    let mut json = String::new();
    io::stdin().read_to_string(&mut json).unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&json).unwrap();
    let native = envelope["KernelBrowser"]["command"]["command"].clone();
    match serde_json::from_value::<command::KernelComputerCommand>(native) {
        Ok(command::KernelComputerCommand::DisplaySubscribe { target, codecs }) => {
            println!("{}", serde_json::json!({"target": target, "codecs": codecs}));
        }
        result => {
            eprintln!("native subscribe deserialization failed: {result:?}");
            std::process::exit(1);
        }
    }
}
