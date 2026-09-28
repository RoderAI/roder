//! A static server for a MiniWoB++ checkout's `miniwob/html/` folder.
//!
//! Only GETs of files under the root are answered; a path with `..`, a
//! backslash or anything outside plain file-name characters is a 404, so
//! nothing outside the checkout can be read. Same plain HTTP/1.1, one request
//! per connection, as the fixture site.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use crate::fixture_harness::site::read_request;

pub(super) struct StaticSite {
    origin: String,
}

impl StaticSite {
    /// Bind `127.0.0.1:0` and serve `root` until the test's runtime ends.
    pub(super) async fn start(root: PathBuf) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let root = root.clone();
                tokio::spawn(async move { serve(stream, &root).await });
            }
        });
        Ok(Self { origin })
    }

    /// The start URL of one task page.
    pub(super) fn task_url(&self, task: &str) -> String {
        format!("{}/miniwob/{task}.html", self.origin)
    }
}

async fn serve(mut stream: TcpStream, root: &Path) {
    let Some((method, path, _)) = read_request(&mut stream).await else {
        return;
    };
    let file = (method == "GET").then(|| resolve(root, &path)).flatten();
    let found = match file {
        Some(file) => tokio::fs::read(&file).await.ok().map(|body| (file, body)),
        None => None,
    };
    let (status, kind, body) = match found {
        Some((file, body)) => ("200 OK", content_type(&file), body),
        None => ("404 Not Found", "text/plain", b"not found".to_vec()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\
         cache-control: no-store\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

/// The file a request path names under `root`, if it is a safe one.
fn resolve(root: &Path, path: &str) -> Option<PathBuf> {
    let path = path.split(['?', '#']).next()?.strip_prefix('/')?;
    let safe = !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != ".."
                && part != "."
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        });
    safe.then(|| root.join(path)).filter(|file| file.is_file())
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_paths_under_the_root_resolve() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            resolve(&root, "/Cargo.toml?x=1"),
            Some(root.join("Cargo.toml"))
        );
        assert!(resolve(&root, "/src/lib.rs").is_some());
        for bad in [
            "/../Cargo.toml",
            "/src/../Cargo.toml",
            "//etc/passwd",
            "/src",
            "/",
            "Cargo.toml",
            "/missing.html",
            "/src/./lib.rs",
            "/a%2e%2e/b",
        ] {
            assert_eq!(resolve(&root, bad), None, "{bad}");
        }
    }
}
