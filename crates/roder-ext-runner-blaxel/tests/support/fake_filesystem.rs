//! Canonical binary filesystem protocol; shared contract fixtures store UTF-8 examples.
use super::*;
pub(super) fn route(
    state: &Arc<Mutex<ServerState>>,
    method: &str,
    path: &str,
    request: &str,
) -> (&'static str, String) {
    let key = path.trim_start_matches("/filesystem/");
    match method {
        "PUT" => {
            let (_, body) = request.split_once("\r\n\r\n").unwrap();
            if serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .is_some_and(|input| input["isDirectory"] == true)
            {
                return ("200 OK", serde_json::json!({"message":"ok"}).to_string());
            }
            let marker = "name=\"file\";";
            let file = body.split_once(marker).expect("binary file part").1;
            let file = file.split_once("\r\n\r\n").unwrap().1;
            let boundary = body.lines().next().unwrap();
            let content = file.split_once(&format!("\r\n{boundary}")).unwrap().0;
            state
                .lock()
                .unwrap()
                .files
                .insert(key.into(), content.into());
            ("200 OK", serde_json::json!({"message":"ok"}).to_string())
        }
        "GET" => {
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("accept: application/octet-stream")
            );
            match state.lock().unwrap().files.get(key) {
                Some(content) => ("200 OK", content.clone()),
                None => ("404 Not Found", json_error("file not found")),
            }
        }
        "DELETE" => {
            state.lock().unwrap().files.remove(key);
            ("200 OK", serde_json::json!({"message":"ok"}).to_string())
        }
        _ => ("404 Not Found", json_error("filesystem route unavailable")),
    }
}
