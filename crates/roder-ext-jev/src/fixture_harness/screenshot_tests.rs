//! Jev's own screenshots on real Chrome, including native high-DPI pixels.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::ImageFormat;
use serde_json::json;

use crate::engine::JevBrowser;

#[tokio::test]
async fn jev_screenshots_preserve_css_coordinates_at_retina_scale_and_after_scroll() {
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    page.call(
        "Emulation.setDeviceMetricsOverride",
        json!({"width":817,"height":493,"deviceScaleFactor":2,"mobile":false}),
    )
    .await
    .unwrap();
    // Headless Chrome needs its tab active to draw the screenshot's frame.
    page.activate().await.unwrap();
    page.evaluate("(() => {
        document.body.style.height = '2400px';
        const marker = document.createElement('div');
        marker.style.cssText = 'position:absolute;left:20px;top:1040px;width:80px;height:80px;background:rgb(255,0,0)';
        document.body.append(marker); scrollTo(0,1000); return true;
    })()").await.unwrap();
    let before = page
        .evaluate("[innerWidth,innerHeight,scrollY,devicePixelRatio]")
        .await
        .unwrap();
    assert_eq!(before, json!([817, 493, 1000, 2]));
    for _ in 0..3 {
        let url = page.screenshot().await.unwrap().unwrap();
        let bytes = STANDARD
            .decode(url.strip_prefix("data:image/jpeg;base64,").unwrap())
            .unwrap();
        let image = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8();
        assert_eq!(image.dimensions(), (817, 493));
        let pixel = image.get_pixel(40, 60).0;
        assert!(
            pixel[0] > 240 && pixel[1] < 15 && pixel[2] < 15,
            "The screenshot must show the scrolled marker at its CSS coordinates: {pixel:?}"
        );
        assert_eq!(
            page.evaluate("[innerWidth,innerHeight,scrollY,devicePixelRatio]")
                .await
                .unwrap(),
            before
        );
    }
    page.close().await.unwrap();
}
