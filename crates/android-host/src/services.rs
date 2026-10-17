//! The platform services the Android host gives the UI (`lightcraft_ui_egui::Services`).
//!
//! For now the ones that need no Android API: writing files (exports, `ui.screenshot`, presets),
//! encoding PNGs and naming the log file. Pickers, sharing, the clipboard and MediaStore come with
//! their cards (APK-22 on), each as a `Services` field.

use std::path::Path;
use std::sync::Arc;

use lightcraft_ui_egui::Services;

/// The services every Android start has; `log_file` is the log being written, if any.
pub fn base_services(log_file: Option<&Path>) -> Services {
    Services {
        write: Some(Box::new(lightcraft_engine::export::write_file)),
        // with it, exports started from the UI run in the background
        write_shared: Some(Arc::new(lightcraft_engine::export::write_file)),
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        log_file: log_file.map(|p| p.display().to_string()),
        ..Services::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn services_have_writer_and_png() {
        let mut s = base_services(Some(Path::new("/data/user/0/ai.storyteller.lightcraft/files/logs/lightcraft.log")));
        assert_eq!(s.log_file.as_deref(), Some("/data/user/0/ai.storyteller.lightcraft/files/logs/lightcraft.log"));
        assert!(base_services(None).log_file.is_none());

        let dir = std::env::temp_dir().join(format!("lc-android-services-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("export.jpg");
        let target = target.to_string_lossy().to_string();
        (s.write.as_mut().unwrap())(&target, b"one").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"one");
        (s.write_shared.as_ref().unwrap())(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");

        let img = lightcraft_raster::Rgba8::from_bytes(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let png = (s.png.as_ref().unwrap())(&img);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "a PNG file");
        // nothing a desktop-only picker would need
        assert!(s.picker.is_none() && s.pick_files.is_none() && s.reveal.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
