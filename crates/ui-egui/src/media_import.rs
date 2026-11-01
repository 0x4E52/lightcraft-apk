//! File ▸ Import from Photo Library… (`file.importMediaLibrary`, APK-23): the device's photo
//! library as folders, each imported by **path** through the import review, exactly as a folder
//! the user picked (`file.addFolder`).
//!
//! The host gives [`crate::Services::import_media`] (Android: a MediaStore query). The command
//! calls it on a worker thread, which holds the activity row "Reading the photo library" for the
//! whole query (a 20k-photo library takes a while), and [`poll`] takes the answer in `logic`: the
//! import review opens with the library's folders as its source, and choosing one
//! (`{bucket: path}`) scans that folder's own photos into the review (not its subfolders: each is a
//! folder of its own in the list), keeping the list to choose another.
//!
//! Photos added in place from the photo library are other apps' files: Android lets LightCraft
//! read them but not write next to them, so XMP sidecars, rename, move and delete fail there. A
//! host that says so ([`crate::Services::prefer_copy_import`]) gets Copy as the review's default,
//! a note under Add in place, and [`read_only_original_error`] on the failures.

use std::sync::mpsc::{Receiver, TryRecvError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;

/// The activity row of the query.
pub const LABEL: &str = "Reading the photo library";

/// What the review says under Add in place where originals are read-only to LightCraft.
pub const ADD_IN_PLACE_NOTE: &str = "Photos added in place stay in the phone's photo library, where LightCraft can't write: XMP sidecars, rename, move and delete won't work on them. Copy keeps them editable.";

/// A folder of the device's photo library (Android: a MediaStore bucket).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaBucket {
    /// The name the system's gallery gives it ("Camera", "Screenshots").
    pub name: String,
    /// The folder; choosing it scans the photos directly in it, not its subfolders'.
    pub path: String,
    /// Photos in it.
    pub count: usize,
    /// Their size in bytes.
    pub bytes: u64,
}

/// Lists the photo library's folders; called on a worker thread (the query may take seconds).
pub type ImportMedia = std::sync::Arc<dyn Fn() -> Result<Vec<MediaBucket>, String> + Send + Sync>;

/// A query on its worker, and its activity row.
pub struct MediaQuery {
    rx: Receiver<Result<Vec<MediaBucket>, String>>,
    _guard: lightcraft_engine::activity::TaskGuard,
}

/// `file.importMediaLibrary` `{bucket?}`: with a folder, scan it into the review (the folder list
/// shown so far stays); without, read the library's folders in the background.
pub(crate) fn run(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    match p.get("bucket") {
        None | Some(Value::Null) => start(app),
        Some(Value::String(path)) if !path.trim().is_empty() => {
            let buckets = match &app.ui.dialog {
                Some(crate::state::Dialog::Import { opts }) => opts.buckets.clone(),
                _ => Vec::new(),
            };
            open_bucket(app, path, buckets)
        }
        Some(other) => Err(format!("file.importMediaLibrary: `bucket` must be a folder path, not {other}")),
    }
}

/// Start the query on a worker; `Err` without a host query, or while one runs.
fn start(app: &mut LightcraftApp) -> Result<Value, String> {
    let Some(query) = app.services.import_media.clone() else {
        return Err("Import from Photo Library is not available here".into());
    };
    if app.media_query.is_some() {
        return Err("The photo library is already being read".into());
    }
    let guard = app.session.activity.start("import", LABEL, lightcraft_engine::activity::Cancel::No);
    let (tx, rx) = std::sync::mpsc::channel();
    let repaint = app.tasks.repaint.clone();
    let job = move || {
        // a panic in the host's query is an error, never a dead worker with a row left behind
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| query()))
            .unwrap_or_else(|_| Err("Reading the photo library failed unexpectedly".to_owned()));
        let _ = tx.send(answer);
        if let Some(ctx) = repaint {
            ctx.request_repaint();
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("media-query".into()).spawn(job).map_err(|e| format!("Can't read the photo library: {e}"))?;
    #[cfg(target_arch = "wasm32")]
    job();
    app.media_query = Some(MediaQuery { rx, _guard: guard });
    Ok(json!({"reading": true}))
}

/// Scan `path`'s own photos into the import review, keeping `buckets` as its source list.
fn open_bucket(app: &mut LightcraftApp, path: &str, buckets: Vec<MediaBucket>) -> Result<Value, String> {
    let r = crate::import::open_folder_only(app, path);
    if r.is_ok()
        && let Some(t) = &mut app.scan
    {
        t.buckets = buckets;
    }
    r
}

/// Take the query's answer (every frame): the review opens with the folders, or the error shows.
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(query) = &app.media_query else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let answer = match query.rx.try_recv() {
        Ok(answer) => answer,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("Reading the photo library failed".to_owned()),
    };
    // (the row goes with the query)
    app.media_query = None;
    match answer {
        Ok(buckets) if buckets.is_empty() => {
            app.ui.status = crate::i18n::tr("No photos in the photo library").to_string();
            app.toast(ctx, crate::i18n::tr("No photos in the photo library"));
        }
        Ok(buckets) => {
            let mut d = crate::import::ImportDialog::new(Vec::new());
            d.buckets = buckets;
            d.copy = app.services.prefer_copy_import;
            app.ui.dialog = Some(crate::state::Dialog::Import { opts: Box::new(d) });
        }
        Err(e) => {
            log::warn!("file.importMediaLibrary: {e}");
            app.ui.status = e.clone();
            app.toast(ctx, e);
        }
    }
}

/// The folder list of the review: one button per folder; the one being shown is active.
pub(crate) fn bucket_list(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &crate::import::ImportDialog) {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, b) in d.buckets.iter().enumerate() {
            let on = d.sources.len() == 1 && d.sources[0] == b.path;
            let label = format!("{} · {}", b.name, b.count);
            if crate::widgets::text_button(ui, &format!("importBucket:{i}"), &label, on).on_hover_text(&b.path).clicked() {
                chosen = Some(b.path.clone());
            }
        }
    });
    if let Some(path) = chosen
        && let Err(e) = open_bucket(app, &path, d.buckets.clone())
    {
        app.toast(ui.ctx(), e);
    }
}

/// The commands that write next to, rename, move or delete originals: only their failures can be
/// an added-in-place photo of the photo library (an export or a library write that is refused is
/// another failure, and keeps its own text).
pub const ORIGINAL_COMMANDS: &[&str] = &[
    "photo.saveMetadataToFile",
    "photo.rename",
    "photo.moveToFolder",
    "photo.delete",
    "photo.deletePermanently",
    "library.emptyRecentlyDeleted",
    "folder.rename",
    "folder.move",
];

/// Where originals are read-only to LightCraft (`prefer_copy_import`): a refused write of a
/// command on originals ([`ORIGINAL_COMMANDS`]) says to import with Copy, both as the command's
/// error and in the per-photo `failed[].error` of a result (Save Metadata to File).
pub(crate) fn tell_read_only_originals(id: &str, r: &mut Result<Value, crate::CommandError>) {
    if !ORIGINAL_COMMANDS.contains(&id) {
        return;
    }
    match r {
        Err(e) => {
            if let Some(message) = read_only_original_error(&e.message) {
                e.message = message;
            }
        }
        Ok(v) => {
            for f in v.get_mut("failed").and_then(Value::as_array_mut).into_iter().flatten() {
                if let Some(error) = f.get_mut("error")
                    && let Some(message) = error.as_str().and_then(read_only_original_error)
                {
                    *error = Value::String(message);
                }
            }
        }
    }
}

/// A command's failure on an original LightCraft may not change (Android: a photo of another
/// app, added in place), told as what to do instead; `None` when `message` is another failure.
/// The engine reports the file and the system's error text (`…: Permission denied (os error
/// 13)`), so this reads that text.
pub fn read_only_original_error(message: &str) -> Option<String> {
    let lower = message.to_lowercase();
    let refused = ["permission denied", "os error 13", "read-only file system", "os error 30", "operation not permitted"]
        .iter()
        .any(|needle| lower.contains(needle));
    refused.then(|| {
        format!(
            "{}: {}",
            message.trim_end_matches('.'),
            crate::i18n::tr(
                "LightCraft can't change this file: it belongs to the phone's photo library. Import the photo with Copy to write XMP, rename, move or delete it."
            )
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The write next to an added-in-place original fails as the system refuses it (an injected
    /// `PermissionDenied`: tests run as root here, which ignores read-only modes), and the error
    /// says what to do.
    #[test]
    fn add_mode_write_beside_original_gives_actionable_error() {
        let path = "/storage/emulated/0/DCIM/Camera/IMG_1.xmp";
        let e = std::io::Error::from_raw_os_error(13);
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        // as the engine words it: the file, then the system's text
        let message = format!("{path}: {e}");
        let told = read_only_original_error(&message).unwrap();
        assert!(told.starts_with(path), "{told}");
        assert!(told.contains("Import the photo with Copy"), "{told}");
        // the kind alone, and a read-only volume, read the same
        assert!(read_only_original_error(&std::io::Error::from(std::io::ErrorKind::PermissionDenied).to_string()).is_some());
        assert!(read_only_original_error(&format!("{path}: {}", std::io::Error::from_raw_os_error(30))).is_some());
        // other failures keep their own text
        assert_eq!(read_only_original_error(&format!("{path}: {}", std::io::Error::from_raw_os_error(28))), None);
        assert_eq!(read_only_original_error("IMG_1.jpg is a virtual copy: its settings live only in the library"), None);
        // only commands on originals are told so: a refused export keeps its own text
        let refused = || Err(crate::CommandError::from(lightcraft_engine::EngineError::Other(message.clone())));
        let mut r = refused();
        tell_read_only_originals("photo.rename", &mut r);
        assert!(r.unwrap_err().message.contains("Import the photo with Copy"));
        let mut r = refused();
        tell_read_only_originals("export.contactSheet", &mut r);
        assert_eq!(r.unwrap_err().message, message);
        let mut r = Ok(json!({"written": [], "failed": [{"id": 1, "error": message}]}));
        tell_read_only_originals("photo.saveMetadataToFile", &mut r);
        assert!(r.unwrap()["failed"][0]["error"].as_str().unwrap().contains("Import the photo with Copy"));
    }
}
