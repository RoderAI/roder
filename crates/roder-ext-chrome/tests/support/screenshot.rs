// A screenshot must not briefly change the page's viewport on Retina displays.
async fn screenshot_preserves_viewport(direct: &mut DirectSession, registry: &ToolRegistry) {
    let original = eval(
        registry,
        "(() => {
            window.screenshotViewChanges = [];
            window.screenshotViewChanged = () => window.screenshotViewChanges.push({
                width: innerWidth, height: innerHeight,
                visualWidth: visualViewport.width, visualHeight: visualViewport.height,
                scale: visualViewport.scale
            });
            window.addEventListener('resize', window.screenshotViewChanged);
            visualViewport.addEventListener('resize', window.screenshotViewChanged);
            return [innerWidth, innerHeight, scrollX, scrollY];
        })()",
    )
    .await;
    for _ in 0..3 {
        let picture = direct.run("screenshot", &json!({})).await;
        assert!(!picture.is_error, "{}", picture.text);
        let size = jpeg_size(picture.image.as_deref().unwrap());
        assert_eq!(json!([size.0, size.1]), json!([original[0], original[1]]));
    }
    // Flush resize notifications queued by the final capture.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let changes = eval(
        registry,
        "(() => {
            window.removeEventListener('resize', window.screenshotViewChanged);
            visualViewport.removeEventListener('resize', window.screenshotViewChanged);
            return {changes: window.screenshotViewChanges,
                view: [innerWidth, innerHeight, scrollX, scrollY]};
        })()",
    )
    .await;
    assert_eq!(changes["view"], original, "Capture moved the viewport");
    assert_eq!(
        changes["changes"],
        json!([]),
        "Capture briefly resized the page"
    );

    // Without an explicit CDP clip the capture must still show the scrolled
    // viewport, rather than the top of the document or the entire page.
    eval(
        registry,
        "(() => {
            const spacer = document.createElement('div');
            spacer.id = 'screenshot-spacer'; spacer.style.height = '2400px';
            const marker = document.createElement('div');
            marker.id = 'screenshot-marker';
            marker.style.cssText = 'position:absolute;left:20px;top:1040px;width:80px;height:80px;background:rgb(255,0,0)';
            document.body.append(spacer, marker); scrollTo(0,1000); return scrollY;
        })()",
    )
    .await;
    let picture = direct.run("screenshot", &json!({})).await;
    assert!(!picture.is_error, "{}", picture.text);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(picture.image.as_ref().unwrap().split_once(',').unwrap().1)
        .unwrap();
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
        .unwrap()
        .to_rgb8();
    let pixel = image.get_pixel(40, 60).0;
    assert!(
        pixel[0] > 240 && pixel[1] < 15 && pixel[2] < 15,
        "Screenshot must show the marker at its scrolled viewport coordinates: {pixel:?}"
    );
    assert_eq!(eval(registry, "scrollY").await, 1000);
    eval(
        registry,
        "document.getElementById('screenshot-spacer').remove(); document.getElementById('screenshot-marker').remove(); scrollTo(0,0); true",
    )
    .await;
}
