use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_UPLOAD_FILES: usize = 20;
const MAX_UPLOAD_PATH_BYTES: usize = 4_096;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<PathBuf>", into = "Vec<PathBuf>")]
pub(crate) struct BrowserUploadFiles {
    paths: Vec<PathBuf>,
}

impl std::fmt::Debug for BrowserUploadFiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserUploadFiles")
            .field("file_count", &self.paths.len())
            .finish()
    }
}

impl TryFrom<Vec<PathBuf>> for BrowserUploadFiles {
    type Error = String;

    fn try_from(paths: Vec<PathBuf>) -> Result<Self, Self::Error> {
        Self::new(paths)
    }
}

impl From<BrowserUploadFiles> for Vec<PathBuf> {
    fn from(files: BrowserUploadFiles) -> Self {
        files.paths
    }
}

impl BrowserUploadFiles {
    pub(crate) fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        if paths.is_empty() || paths.len() > MAX_UPLOAD_FILES {
            return Err(format!(
                "browser upload requires 1 through {MAX_UPLOAD_FILES} files"
            ));
        }
        for path in &paths {
            let value = path
                .to_str()
                .ok_or_else(|| "browser upload paths must be valid UTF-8".to_string())?;
            if !path.is_absolute() || value.len() > MAX_UPLOAD_PATH_BYTES || value.contains('\0') {
                return Err("browser upload paths must be bounded absolute paths".to_string());
            }
        }
        Ok(Self { paths })
    }

    pub(crate) fn controller_paths(&self) -> Vec<&str> {
        self.paths.iter().filter_map(|path| path.to_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerDownloadsResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) enabled: bool,
}

impl BrowserControllerDownloadsResult {
    pub(crate) fn validate(&self, target_id: &str, document_id: &str) -> Result<(), String> {
        if self.browser_generation == 0 || !self.enabled {
            return Err("browser controller did not enable downloads".to_string());
        }
        validate_identity(&self.target_id, &self.document_id, target_id, document_id)
    }

    pub(crate) fn into_room_result(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
    ) -> RoomBrowserDownloadsResult {
        RoomBrowserDownloadsResult {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            enabled: self.enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomBrowserDownloadsResult {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BrowserControllerUploadResult {
    pub(crate) browser_generation: u64,
    pub(crate) target_id: String,
    pub(crate) document_id: String,
    pub(crate) file_count: usize,
    pub(crate) total_bytes: u64,
}

impl BrowserControllerUploadResult {
    pub(crate) fn validate(
        &self,
        target_id: &str,
        document_id: &str,
        expected_file_count: usize,
    ) -> Result<(), String> {
        if self.browser_generation == 0 || self.file_count != expected_file_count {
            return Err("browser controller changed the upload file count".to_string());
        }
        validate_identity(&self.target_id, &self.document_id, target_id, document_id)
    }

    pub(crate) fn into_room_result(
        self,
        session_id: String,
        environment_id: String,
        runtime_generation: u64,
        tab_id: String,
        document_revision: u64,
        element_ref: String,
    ) -> RoomBrowserUploadResult {
        RoomBrowserUploadResult {
            session_id,
            environment_id,
            runtime_generation,
            tab_id,
            document_revision,
            element_ref,
            file_count: self.file_count,
            total_bytes: self.total_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomBrowserUploadResult {
    pub(crate) session_id: String,
    pub(crate) environment_id: String,
    pub(crate) runtime_generation: u64,
    pub(crate) tab_id: String,
    pub(crate) document_revision: u64,
    pub(crate) element_ref: String,
    pub(crate) file_count: usize,
    pub(crate) total_bytes: u64,
}

fn validate_identity(
    target_id: &str,
    document_id: &str,
    expected_target_id: &str,
    expected_document_id: &str,
) -> Result<(), String> {
    if target_id != expected_target_id || document_id != expected_document_id {
        return Err("browser controller changed target or document identity".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::BrowserUploadFiles;

    #[test]
    fn upload_files_require_bounded_absolute_utf8_paths() {
        let files = BrowserUploadFiles::new(vec![PathBuf::from("/workspace/report.txt")])
            .expect("safe upload path");
        assert_eq!(files.controller_paths(), vec!["/workspace/report.txt"]);
        assert!(BrowserUploadFiles::new(Vec::new()).is_err());
        assert!(BrowserUploadFiles::new(vec![PathBuf::from("relative.txt")]).is_err());
        assert!(BrowserUploadFiles::new(
            (0..21)
                .map(|index| PathBuf::from(format!("/workspace/{index}")))
                .collect(),
        )
        .is_err());
    }
}
