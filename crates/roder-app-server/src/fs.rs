use std::path::PathBuf;

use base64::Engine;
use roder_protocol::{
    FsReadDirectoryEntry, FsReadDirectoryParams, FsReadDirectoryResponse, FsReadFileParams,
    FsReadFileResponse, JsonRpcError,
};

use crate::AppServer;

impl AppServer {
    pub(crate) async fn handle_fs_read_file(
        &self,
        params: FsReadFileParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let path = absolute_path(&params.path)?;
        let data = tokio::fs::read(&path).await.map_err(internal_error)?;
        Ok(serde_json::to_value(FsReadFileResponse {
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })
        .unwrap())
    }

    pub(crate) async fn handle_fs_read_directory(
        &self,
        params: FsReadDirectoryParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let path = absolute_path(&params.path)?;
        let mut entries = tokio::fs::read_dir(&path).await.map_err(internal_error)?;
        let mut result = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(internal_error)? {
            let metadata = entry.metadata().await.map_err(internal_error)?;
            result.push(FsReadDirectoryEntry {
                file_name: entry.file_name().to_string_lossy().to_string(),
                is_directory: metadata.is_dir(),
                is_file: metadata.is_file(),
            });
        }
        result.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(serde_json::to_value(FsReadDirectoryResponse { entries: result }).unwrap())
    }
}

fn absolute_path(path: &str) -> Result<PathBuf, JsonRpcError> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(JsonRpcError {
            code: -32602,
            message: "path must be absolute".to_string(),
            data: None,
        })
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}
