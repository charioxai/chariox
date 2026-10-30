//! MD-2: sessionless host browser protocol (legacy 417; display 419).
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserCommand {
    /// MP-08/MP-11 local500: human grants only the leased worker's observed desktop.
    GrantLeasedComputer {
        agent_id: String,
        target: KernelDesktopTarget,
    },
    RevokeLeasedComputer {
        agent_id: String,
    },
    Computer {
        command: KernelComputerCommand,
    },
    /// MP-08 / MP-11 (local 461): explicit `null` restores every owned agent after
    /// "revoke all"; the field is required so an omitted agent cannot restore everyone.
    GrantRoomComputer {
        #[serde(deserialize_with = "Option::deserialize")]
        agent_id: Option<String>,
    },
    ListGrants,
    /// Protocol 556: `State` that waits (<= 2000 ms) until the browser state differs from the one
    /// `after` names (the `revision` of the caller's last read; empty reads at once). Replaces a poll.
    StateWait {
        #[serde(default)]
        after: String,
        wait_ms: u32,
    },
    SubscribeGrants {
        after: u64,
        wait_ms: u32,
    },
    RevokeGrants {
        agent_id: Option<String>,
    },
    /// MP-08/MP-10: protocol 482 `wire: 2` selects the author-CSS snapshot
    /// plus delta mirror; absent keeps the 443 computed-style packets.
    MirrorSubscribe {
        tab_id: String,
        generation: u64,
        device_scale_factor: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wire: Option<u32>,
        /// Protocol 550/158: allow protected native video during mirror preparation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        navigation_preview: Option<bool>,
        /// MP-08/MP-10/MP-11: protocol 550 native document font proof.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font_requirements: Option<bool>,
        /// MP-08/MP-10/MP-11: protocol 567/175 asks for the `fonts` op's `absent` list (needs `font_requirements`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        font_absent: Option<bool>,
        /// MP-08/MP-10: protocol 629 viewport-first snapshot: off-screen subtrees follow as continuations, their
        /// elements first travel as `deferred` records that keep the native box, and the header reports
        /// `snapshot_viewport_complete` (needs `wire: 2`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        viewport_first: Option<bool>,
        /// MP-08/MP-10: protocol 629: links the kernel saw visited from the page's origin travel with an empty
        /// `href`, which the viewer draws `:visited` (needs `wire: 2`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visited_links: Option<bool>,
        /// Protocol 560/168: the person's display in CSS px; the Tab lays out at this size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout_width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout_height: Option<u32>,
    },
    /// MP-08/MP-10: protocol 482 `wait_ms` (<= 2000) makes a v2 credit a long
    /// poll answered by the next page change; credits apply in sequence.
    MirrorNext {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
        #[serde(default)]
        drift_nodes: Vec<String>,
        /// Protocol 550/158: immutable sheet digests
        /// the viewer retains for this subscription (<= 128, each 24 hex chars).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        known_sheets: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait_ms: Option<u32>,
        /// Protocol 501: the viewer lacks the packet after `after_sequence` (a reply
        /// lost with its socket, or a failed credit): the controller resends it from
        /// its history at once, ending earlier long polls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resend: Option<bool>,
        /// End earlier long polls and drain freshly after acknowledged preview input.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flush: Option<bool>,
    },
    MirrorClose {
        subscription_id: String,
        generation: u64,
    },
    MirrorInput {
        tab_id: String,
        generation: u64,
        document_id: String,
        subscription_id: String,
        sequence: u64,
        action: KernelBrowserMirrorAction,
    },
    DisplayCapture {
        tab_id: String,
        generation: u64,
    },
    DisplayTakeover {
        tab_id: String,
        generation: u64,
    },
    DisplayRelease {
        tab_id: String,
        generation: u64,
    },
    DisplayActors,
    DisplaySubscribe {
        tab_id: String,
        generation: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<KernelBrowserDisplayTarget>,
        codecs: Vec<String>,
        bitrate: u32,
        device_scale_factor: u32,
        /// MP-08/MP-10 allocated 540 (PERF_AUDIT 4.2.3): the viewer holds frames pushed before its
        /// relay registration response; the push pump grants the first frame at registration.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        initial_push: bool,
        /// MP-08/MP-10/MP-11 allocated 720 (P1): one page display for the Tab's life. Its source and lease
        /// follow each committed document of the Tab; the viewer parks it while the DOM mirror paints.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        tab_display: bool,
    },
    DisplayNext {
        subscription_id: String,
        generation: u64,
        after_sequence: u64,
    },
    /// MP-08/MP-10/MP-11: protocol 525. A resume hint does not carry authority;
    /// current terminal/source admission is repeated before a fresh stream ID.
    DisplaySuspend {
        subscription_id: String,
        generation: u64,
    },
    DisplayResume {
        subscription_id: String,
        generation: u64,
    },
    /// MP-08/MP-10 allocated 720 (P1): park (no frames, no readback) or unpark a Tab display; the Tab's
    /// next commit unparks it in the kernel.
    DisplayPark {
        subscription_id: String,
        generation: u64,
        parked: bool,
    },
    /// MP-08/MP-10: protocol 491 pushed display. Acknowledges the newest
    /// presented sequence on a relay display subscription (the first ack
    /// starts the kernel push pump); `lost` requests an independent frame.
    DisplayAck {
        subscription_id: String,
        generation: u64,
        sequence: u64,
        lost: bool,
        /// MP-08/MP-10/MP-11: protocol 536 receiver hold time, excluded from
        /// the pump's wire RTT. Absent keeps legacy gate semantics.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        held_ms: Option<u32>,
        /// Allocated 919: nobody watches this display. The pump stops capturing and sends nothing, not even a
        /// reprobe, while the acknowledgement renews the delivery lease; only an acknowledgement that asks for
        /// an independent frame (`lost`) resumes it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        paused: bool,
    },
    DisplayInput {
        tab_id: String,
        generation: u64,
        document_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sequence: Option<u64>,
        input: KernelBrowserInput,
    },
    Start,
    State,
    Stop,
    Open {
        url: String,
    },
    Close {
        tab_id: String,
        generation: u64,
    },
    /// MP-08/MP-10: protocol 629 live layout mutation, preserving mirror attachments.
    Resize {
        tab_id: String,
        generation: u64,
        width: u32,
        height: u32,
    },
    Navigate {
        tab_id: String,
        generation: u64,
        url: String,
    },
    Snapshot {
        tab_id: String,
        generation: u64,
    },
    Input {
        tab_id: String,
        generation: u64,
        input: KernelBrowserInput,
    },
    /// Protocol 630/233: answer the current kernel-owned page dialog.
    Dialog {
        tab_id: String,
        generation: u64,
        document_id: String,
        dialog_id: String,
        action: KernelBrowserDialogAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_text: Option<String>,
    },
    Screenshot {
        tab_id: String,
        generation: u64,
    },
    Unsubscribe {
        subscription_id: String,
        generation: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelBrowserDialogAction {
    Accept,
    Dismiss,
}

/// MP-08/MP-10/MP-11: allocated519; only owner-issued references select regions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserDisplayTarget {
    Region {
        mirror_subscription_id: String,
        document_id: String,
        sequence: u64,
        region_id: String,
    },
    BrowserChrome {
        window_generation: u64,
    },
    /// MP-08/MP-10/MP-11 protocol 530: one popup of the band's window (menu,
    /// omnibox dropdown, extension popup, bubble) by its kernel-issued id from
    /// the Browser state's `chrome_window.auxiliary`.
    BrowserAuxiliary {
        window_generation: u64,
        auxiliary_id: u64,
    },
    /// MP-08/MP-10/MP-11 protocol 560: the owner's view of the Chrome window's page area. Bound to
    /// the window epoch, never to a Tab or document; Tab create, switch and navigation keep it.
    BrowserWindow {
        window_generation: u64,
    },
}

/// MP-08/MP-10/MP-11 protocol 530: non-primary buttons reach only Chrome's own UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelBrowserButton {
    Middle,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserInput {
    Click {
        x: u32,
        y: u32,
        /// Protocol 530: absent means the primary button.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        button: Option<KernelBrowserButton>,
    },
    /// Protocol 530: a primary-button drag (the band's Tab reorder).
    Drag {
        x: u32,
        y: u32,
        to_x: u32,
        to_y: u32,
    },
    Text {
        text: String,
    },
    Key {
        key: String,
        /// MP-08/MP-11 protocol517: bounded CDP modifier bitset (carried from 498).
        #[serde(
            default,
            deserialize_with = "deserialize_key_modifiers",
            skip_serializing_if = "key_modifiers_empty"
        )]
        modifiers: u8,
    },
    Scroll {
        x: u32,
        y: u32,
        delta_x: i32,
        delta_y: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelBrowserRequest {
    pub command: KernelBrowserCommand,
}

/// MP-08/MP-11: element IDs are scoped to a caller-bound sanitized document epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KernelBrowserMirrorAction {
    /// Protocol 482: `x`/`y` are the viewer's CSS offset inside the node's box;
    /// the kernel clamps them to the live box and hit-tests before dispatch.
    Click {
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    Focus {
        node_id: String,
    },
    /// Protocol 482: without `node_id`, text goes to the live native focus.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
        text: String,
    },
    Scroll {
        node_id: String,
        delta_x: i32,
        delta_y: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
    },
    /// Protocol 482 viewer-owned scroll: the kernel follows the viewer's
    /// position (window when `node_id` is absent).
    ScrollTo {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
        x: f64,
        y: f64,
    },
    Key {
        key: String,
    },
    Selection {
        anchor_id: String,
        anchor_offset: u32,
        focus_id: String,
        focus_offset: u32,
    },
    /// Protocol 492: the viewer's focus left this control; the kernel blurs it
    /// only while it is still the live focus.
    Blur {
        node_id: String,
    },
    /// Protocol 492: a control state committed by the viewer's own UI (a select
    /// pick, a date/time/color/range picker, a multiple select's option).
    Set {
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checked: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected_index: Option<i32>,
    },
    Coordinate {
        input: KernelBrowserInput,
    },
}

fn key_modifiers_empty(value: &u8) -> bool {
    *value == 0
}
fn deserialize_key_modifiers<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u8, D::Error> {
    let value = u8::deserialize(deserializer)?;
    if value > 15 {
        return Err(serde::de::Error::custom("invalid key modifiers"));
    }
    Ok(value)
}

/// MP-08/MP-10/MP-11: kernel-private §2a target, ready for the 519 adapter.
/// This type is not yet a command field; adding it to the public request belongs
/// to the coordinated 519/127 protocol bump. Clients never choose crop geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelBrowserMirrorRegionTarget {
    pub mirror_subscription_id: String,
    pub generation: u64,
    pub document_id: String,
    pub sequence: u64,
    pub region_id: String,
}
