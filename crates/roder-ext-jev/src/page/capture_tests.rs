//! Check the wire contract: page resize events cannot detect Chrome's internal
//! capture-time view resizing caused by device emulation on a Retina display.
use std::{
    io::Cursor,
    sync::{Arc, Mutex},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{SinkExt, StreamExt};
use image::ImageFormat;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::Page;
use crate::{cdp::Connection, engine::JevBrowser};

async fn exercise(version: Value, capture: bool) -> Vec<Value> {
    let image = image::RgbImage::from_pixel(160, 120, image::Rgb([255, 0, 0]));
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).unwrap();
    let png = STANDARD.encode(png.into_inner());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("ws://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "Target.createTarget" => json!({"targetId":"page"}),
                "Target.attachToTarget" => json!({"sessionId":"session"}),
                "Browser.getVersion" => version.clone(),
                "Runtime.evaluate" => json!({"result":{"value":[80,60]}}),
                "Page.captureScreenshot" => json!({"data":png}),
                _ => json!({}),
            };
            recorded.lock().unwrap().push(request.clone());
            let response = json!({"id":request["id"],"result":result});
            if socket
                .send(Message::Text(response.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let connection = Connection::connect(&endpoint).await.unwrap();
    let mut page = Page::create(connection, false).await.unwrap();
    page.prepare().await.unwrap();
    if capture {
        assert!(
            requests.lock().unwrap().iter().all(|request| !matches!(
                request["method"].as_str(),
                Some(
                    "Emulation.setDeviceMetricsOverride"
                        | "Emulation.clearDeviceMetricsOverride"
                        | "Page.bringToFront"
                        | "Target.activateTarget"
                )
            )),
            "A windowed Jev tab must retain its native viewport"
        );
        for _ in 0..3 {
            let data = page.screenshot().await.unwrap().unwrap();
            let bytes = STANDARD
                .decode(data.strip_prefix("data:image/jpeg;base64,").unwrap())
                .unwrap();
            let jpeg = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
            assert_eq!(
                (jpeg.width(), jpeg.height()),
                (80, 60),
                "Retina screenshots must use CSS coordinates"
            );
        }
    }
    server.abort();
    let recorded = requests.lock().unwrap().clone();
    recorded
}

#[tokio::test]
async fn windowed_jev_capture_never_overrides_or_resizes_the_viewport() {
    let requests = exercise(
        json!({"product":"Chrome/150", "userAgent":"Chrome/150"}),
        true,
    )
    .await;
    let captures: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "Page.captureScreenshot")
        .collect();
    assert_eq!(captures.len(), 3);
    for capture in captures {
        assert!(capture["params"].get("clip").is_none());
        assert_eq!(capture["params"]["captureBeyondViewport"], false);
    }
}

#[tokio::test]
async fn headless_jev_keeps_its_fixed_viewport_for_both_version_shapes() {
    for version in [
        json!({"product":"HeadlessChrome/150", "userAgent":"Chrome/150"}),
        json!({"product":"Chrome/150", "userAgent":"HeadlessChrome/150"}),
    ] {
        let requests = exercise(version, false).await;
        let metrics = requests
            .iter()
            .find(|request| request["method"] == "Emulation.setDeviceMetricsOverride")
            .unwrap();
        assert_eq!(
            metrics["params"],
            json!({"width":1120, "height":780, "deviceScaleFactor":1, "mobile":false})
        );
    }
}
