//! A headed CDP capture must not ask Chrome to resize or activate its view.
use std::{io::Cursor, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures::{SinkExt, StreamExt};
use image::ImageFormat;
use roder_ext_chrome::direct::{DirectSession, DirectTab, OpenGuard};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn screenshot_keeps_chrome_view_unchanged_and_returns_css_pixels() {
    // Native Retina surface: twice the CSS viewport in each direction.
    let source = image::RgbImage::from_pixel(160, 120, image::Rgb([0, 0, 0]));
    let mut png = Cursor::new(Vec::new());
    source.write_to(&mut png, ImageFormat::Png).unwrap();
    let image = STANDARD.encode(png.into_inner());
    let requests = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let recorded = requests.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tab = DirectTab::Page {
        websocket: format!("ws://{}", listener.local_addr().unwrap()),
        target_id: "retina-page".into(),
    };
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "Browser.getVersion" => json!({"product":"Chrome/150"}),
                "Page.getFrameTree" => json!({"frameTree":{"frame":{"id":"frame"}}}),
                "Page.createIsolatedWorld" => json!({"executionContextId":1}),
                "Runtime.evaluate" => {
                    let expression = request["params"]["expression"].as_str().unwrap();
                    let value = if expression == "location.href" {
                        json!("https://fixture.test/")
                    } else if expression.contains("innerWidth")
                        && !expression.contains("__roderDirect")
                    {
                        // Also answers the old viewport query, so the regression
                        // fails on the resizing request rather than setup.
                        if expression.contains("scrollX") {
                            json!([0, 0, 80, 60, 2])
                        } else {
                            json!([80, 60])
                        }
                    } else if expression.ends_with(".shown()") {
                        json!("")
                    } else {
                        json!(0)
                    };
                    json!({"result":{"value":value}})
                }
                "Page.captureScreenshot" => json!({"data":image}),
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
    let mut session = DirectSession::attach(&tab, Arc::new(OpenGuard), false)
        .await
        .unwrap();
    let result = session.run("screenshot", &json!({})).await;
    assert!(!result.is_error, "{}", result.text);
    let requests = requests.lock().unwrap();
    let capture = requests
        .iter()
        .find(|request| request["method"] == "Page.captureScreenshot")
        .unwrap();
    // Chromium temporarily changes its view size whenever a clip is supplied,
    // even if page JS never receives a resize event. Check the wire request.
    assert!(
        capture["params"].get("clip").is_none(),
        "Capture must not resize Chrome: {capture}"
    );
    assert_eq!(capture["params"]["captureBeyondViewport"], false);
    assert!(requests.iter().all(|request| !matches!(
        request["method"].as_str(),
        Some("Page.bringToFront" | "Emulation.setDeviceMetricsOverride")
    )));
    let image = result.image.unwrap();
    let bytes = STANDARD
        .decode(image.strip_prefix("data:image/jpeg;base64,").unwrap())
        .unwrap();
    let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (80, 60));
    assert_eq!(
        (
            result.data["width"].as_u64(),
            result.data["height"].as_u64()
        ),
        (Some(80), Some(60))
    );
    server.abort();
}
