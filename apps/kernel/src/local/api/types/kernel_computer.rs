//! MP-08 / MP-11: local446 / peer89 owned-desktop contract, shared by MCP/terminals.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelDesktopTarget {
    pub surface_id: String,
    pub generation: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelComputerCommand {
    Start {},
    State {},
    Actors {},
    Snapshot {
        target: KernelDesktopTarget,
    },
    Screenshot {
        target: KernelDesktopTarget,
    },
    Ocr {
        target: KernelDesktopTarget,
        query: Option<String>,
    },
    ClipboardRead {
        target: KernelDesktopTarget,
    },
    Input {
        target: KernelDesktopTarget,
        input: KernelComputerInput,
    },
    TargetAction {
        target: KernelDesktopTarget,
        tree_revision: u64,
        target_id: String,
        action: String,
    },
    Takeover {
        target: KernelDesktopTarget,
    },
    Release {
        target: KernelDesktopTarget,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelComputerInput {
    Text {
        text: RoomEnvironmentKeyboardInput,
    },
    /// Committed IME text is separate from physical layout/key events. Preedit
    /// stays in the client/input method; no keystroke synthesis from preedit.
    Composition {
        text: RoomEnvironmentKeyboardInput,
    },
    Keycode {
        keycode: u8,
        state: KernelComputerKeyState,
    },
    Key {
        key: RoomEnvironmentKeyboardInput,
    },
    Hold {
        key: RoomEnvironmentKeyboardInput,
        duration_ms: u32,
    },
    PointerHold {
        x: u32,
        y: u32,
        button: u8,
        duration_ms: u32,
    },
    Click {
        x: u32,
        y: u32,
        button: u8,
    },
    Move {
        x: u32,
        y: u32,
    },
    Drag {
        x: u32,
        y: u32,
        to_x: u32,
        to_y: u32,
        button: u8,
    },
    Scroll {
        x: u32,
        y: u32,
        steps: i16,
    },
    ClipboardWrite {
        text: RoomEnvironmentClipboardText,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelComputerKeyState {
    Down,
    Up,
}
