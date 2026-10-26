use super::*;
use roder_protocol::{ThreadListParams, ThreadListResult};

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn handle_session_list(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        self.require_authentication().await?;
        let request: acp::ListSessionsRequest = decode_params(params)?;
        let cwd = request
            .cwd
            .map(|cwd| {
                if !cwd.is_absolute() {
                    return Err(invalid_params("session/list cwd must be absolute"));
                }
                std::fs::canonicalize(cwd).map_err(invalid_params)
            })
            .transpose()?;
        let page: ThreadListResult = self
            .call(
                "thread/list",
                serde_json::to_value(ThreadListParams {
                    limit: Some(50),
                    cursor: request.cursor,
                })
                .map_err(internal_error)?,
            )
            .await?;
        let mut sessions = Vec::new();
        for thread in page.data {
            if cwd
                .as_ref()
                .is_some_and(|cwd| std::fs::canonicalize(&thread.cwd).ok().as_ref() != Some(cwd))
            {
                continue;
            }
            let updated = time::OffsetDateTime::from_unix_timestamp(thread.updated_at)
                .map_err(internal_error)?
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(internal_error)?;
            sessions.push(
                acp::SessionInfo::new(thread.id, thread.cwd)
                    .title(
                        thread
                            .name
                            .or_else(|| (!thread.preview.is_empty()).then_some(thread.preview)),
                    )
                    .updated_at(updated),
            );
        }
        serde_json::to_value(acp::ListSessionsResponse::new(sessions).next_cursor(page.next_cursor))
            .map_err(internal_error)
    }
}
