#![no_main]
use base64::Engine;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let decoded = data.strip_prefix(b"base64:").and_then(|s| {
        base64::engine::general_purpose::STANDARD
            .decode(s.strip_suffix(b"\n").unwrap_or(s))
            .ok()
    });
    let data = decoded.as_deref().unwrap_or(data);
    if let Ok(image) = paint::images::decode_image(data) {
        assert!(image.width <= paint::images::MAX_DIMENSION);
        assert!(image.height <= paint::images::MAX_DIMENSION);
        assert_eq!(
            image.rgba.len(),
            image.width as usize * image.height as usize * 4
        );
    }
});
