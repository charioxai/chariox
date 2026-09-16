use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserPermissionName {
    Camera,
    ClipboardReadWrite,
    ClipboardSanitizedWrite,
    DisplayCapture,
    Geolocation,
    LocalFonts,
    Microphone,
    Midi,
    MidiSysex,
    Notifications,
}

impl BrowserPermissionName {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::ClipboardReadWrite => "clipboard-read-write",
            Self::ClipboardSanitizedWrite => "clipboard-sanitized-write",
            Self::DisplayCapture => "display-capture",
            Self::Geolocation => "geolocation",
            Self::LocalFonts => "local-fonts",
            Self::Microphone => "microphone",
            Self::Midi => "midi",
            Self::MidiSysex => "midi-sysex",
            Self::Notifications => "notifications",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserPermissionSetting {
    Granted,
    Denied,
    Prompt,
}

impl BrowserPermissionSetting {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Prompt => "prompt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerPermissionResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) permission: String,
    pub(crate) setting: String,
}

impl BrowserControllerPermissionResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        document_id: &str,
        permission: BrowserPermissionName,
        setting: BrowserPermissionSetting,
    ) -> Result<(), String> {
        if self.browser_generation == 0 {
            return Err("browser controller permission returned a zero generation".to_string());
        }
        if self.target_id != target_id || self.document_id != document_id {
            return Err("browser controller changed target or document identity".to_string());
        }
        if self.permission != permission.as_str() || self.setting != setting.as_str() {
            return Err("browser controller changed the permission decision".to_string());
        }
        Ok(())
    }

    pub(crate) fn into_room_result(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
    ) -> RoomBrowserPermissionResult {
        RoomBrowserPermissionResult {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            permission: self.permission,
            setting: self.setting,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomBrowserPermissionResult {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) permission: String,
    pub(crate) setting: String,
}

#[cfg(test)]
mod tests {
    use super::{BrowserPermissionName, BrowserPermissionSetting};

    #[test]
    fn permissions_use_closed_first_party_names_and_decisions() {
        assert_eq!(
            [
                BrowserPermissionName::Camera,
                BrowserPermissionName::ClipboardReadWrite,
                BrowserPermissionName::ClipboardSanitizedWrite,
                BrowserPermissionName::DisplayCapture,
                BrowserPermissionName::Geolocation,
                BrowserPermissionName::LocalFonts,
                BrowserPermissionName::Microphone,
                BrowserPermissionName::Midi,
                BrowserPermissionName::MidiSysex,
                BrowserPermissionName::Notifications,
            ]
            .map(BrowserPermissionName::as_str),
            [
                "camera",
                "clipboard-read-write",
                "clipboard-sanitized-write",
                "display-capture",
                "geolocation",
                "local-fonts",
                "microphone",
                "midi",
                "midi-sysex",
                "notifications",
            ]
        );
        assert_eq!(BrowserPermissionSetting::Granted.as_str(), "granted");
        assert_eq!(BrowserPermissionSetting::Denied.as_str(), "denied");
        assert_eq!(BrowserPermissionSetting::Prompt.as_str(), "prompt");
    }
}
