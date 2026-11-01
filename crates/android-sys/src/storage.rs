//! Shared storage: the public folders other apps see, and where LightCraft's exports go (APK-27).
//!
//! [`Folders`] holds the shared volume and its standard folders as
//! `Environment.getExternalStoragePublicDirectory` names them (right for secondary users and work
//! profiles, whose volume is not `/storage/emulated/0`); [`query_folders`] (Android only) asks
//! for them, and [`Folders::fallback`] is the pure answer when JNI fails. Everything else is pure
//! and tested on every target:
//!
//! - [`Folders::export_dir`]: images (JPEG, PNG, TIFF, DNG, HEIF, …) go to `Pictures/LightCraft`,
//!   PDFs, video and any other output to `Documents/LightCraft`. MediaProvider lets an app create
//!   only image (and video) files in `Pictures/` and `DCIM/`, so nothing else may be written there.
//! - [`Folders::media_only`]: `Pictures/` and `DCIM/`, where an XMP sidecar can't be written.
//! - [`Folders::local_roots`]: DCIM, Pictures and Download for the Library sidebar's Local section,
//!   readable in place through FUSE with the photo permission.

use std::path::{Path, PathBuf};

/// The folder LightCraft's exports go into, inside Pictures and Documents.
pub const APP_FOLDER: &str = "LightCraft";

/// `Environment.DIRECTORY_PICTURES`.
pub const DIRECTORY_PICTURES: &str = "Pictures";
/// `Environment.DIRECTORY_DOCUMENTS`.
pub const DIRECTORY_DOCUMENTS: &str = "Documents";
/// `Environment.DIRECTORY_DCIM`.
pub const DIRECTORY_DCIM: &str = "DCIM";
/// `Environment.DIRECTORY_DOWNLOADS`.
pub const DIRECTORY_DOWNLOADS: &str = "Download";

/// The primary shared volume of the device's first user, used when JNI can't name it.
pub const FALLBACK_VOLUME: &str = "/storage/emulated/0";

/// What an export writes, for the folder it goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A photo file MediaProvider files as an image (JPEG, PNG, TIFF, DNG, HEIF, WebP, AVIF, …).
    Image,
    /// A PDF, a video, a text or any other file (and an original with its XMP sidecar).
    Document,
}

/// The shared volume and its public folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// `Environment.getExternalStorageDirectory()`: `/storage/emulated/<user>`.
    pub volume: PathBuf,
    pub pictures: PathBuf,
    pub documents: PathBuf,
    pub dcim: PathBuf,
    pub download: PathBuf,
}

impl Folders {
    /// The standard folders inside `volume`.
    pub fn in_volume(volume: &Path) -> Folders {
        Folders {
            volume: volume.to_path_buf(),
            pictures: volume.join(DIRECTORY_PICTURES),
            documents: volume.join(DIRECTORY_DOCUMENTS),
            dcim: volume.join(DIRECTORY_DCIM),
            download: volume.join(DIRECTORY_DOWNLOADS),
        }
    }

    /// `/storage/emulated/0/…`, for when JNI failed (right for the device's first user).
    pub fn fallback() -> Folders {
        Folders::in_volume(Path::new(FALLBACK_VOLUME))
    }

    /// What JNI answered (`None` where a call failed), each missing or relative folder from
    /// the volume, and the volume from [`FALLBACK_VOLUME`] when it is missing.
    pub fn from_answers(
        volume: Option<PathBuf>,
        pictures: Option<PathBuf>,
        documents: Option<PathBuf>,
        dcim: Option<PathBuf>,
        download: Option<PathBuf>,
    ) -> Folders {
        let usable = |p: Option<PathBuf>| p.filter(|p| p.is_absolute());
        let base = Folders::in_volume(&usable(volume).unwrap_or_else(|| PathBuf::from(FALLBACK_VOLUME)));
        Folders {
            pictures: usable(pictures).unwrap_or(base.pictures),
            documents: usable(documents).unwrap_or(base.documents),
            dcim: usable(dcim).unwrap_or(base.dcim),
            download: usable(download).unwrap_or(base.download),
            volume: base.volume,
        }
    }

    /// `Pictures/LightCraft`: where exported images go.
    pub fn pictures_export(&self) -> PathBuf {
        self.pictures.join(APP_FOLDER)
    }

    /// `Documents/LightCraft`: where PDFs and every other export go.
    pub fn documents_export(&self) -> PathBuf {
        self.documents.join(APP_FOLDER)
    }

    /// The folder an export of `kind` goes to when nothing else names one.
    pub fn export_dir(&self, kind: Kind) -> PathBuf {
        match kind {
            Kind::Image => self.pictures_export(),
            Kind::Document => self.documents_export(),
        }
    }

    /// The two export folders, as `(label, path)`, for the in-app file chooser.
    pub fn export_roots(&self) -> Vec<(String, PathBuf)> {
        vec![("Pictures".to_owned(), self.pictures_export()), ("Documents".to_owned(), self.documents_export())]
    }

    /// The Library sidebar's Local section: DCIM, Pictures and Download.
    pub fn local_roots(&self) -> Vec<(String, PathBuf)> {
        vec![("DCIM".to_owned(), self.dcim.clone()), ("Pictures".to_owned(), self.pictures.clone()), ("Download".to_owned(), self.download.clone())]
    }

    /// The folders that take only image and video files (MediaProvider refuses any other file
    /// created there): `Pictures/` and `DCIM/`.
    pub fn media_only(&self) -> Vec<PathBuf> {
        vec![self.pictures.clone(), self.dcim.clone()]
    }
}

/// `Environment.getExternalStoragePublicDirectory(…)` for each folder and the volume itself, on
/// the calling thread (attached through [`crate::jni_env::with_activity`], so never inside a
/// `with_activity` closure). A folder whose call fails falls back to its place in the volume; when
/// the volume can't be named either the whole answer is [`Folders::fallback`]'s.
#[cfg(target_os = "android")]
pub fn query_folders(app: &android_activity::AndroidApp) -> Result<Folders, crate::Error> {
    crate::jni_env::with_activity(app, |env, _activity| {
        let volume = crate::media_store::primary_volume(env).ok();
        let mut public = |name: &str| public_directory(env, name).ok();
        let (pictures, documents, dcim, download) =
            (public(DIRECTORY_PICTURES), public(DIRECTORY_DOCUMENTS), public(DIRECTORY_DCIM), public(DIRECTORY_DOWNLOADS));
        Ok(Folders::from_answers(volume, pictures, documents, dcim, download))
    })
}

/// `Environment.getExternalStoragePublicDirectory(name).getAbsolutePath()`.
#[cfg(target_os = "android")]
fn public_directory(env: &mut jni::Env<'_>, name: &str) -> Result<PathBuf, crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    env.with_local_frame(16, |env| -> Result<PathBuf, crate::Error> {
        let type_ = env.new_string(name)?;
        let answer = env.call_static_method(
            jni_str!("android/os/Environment"),
            jni_str!("getExternalStoragePublicDirectory"),
            jni_sig!("(Ljava/lang/String;)Ljava/io/File;"),
            &[JValue::Object(&type_)],
        );
        let dir = caught(env, answer)?.l()?;
        if dir.is_null() {
            return Err(crate::Error::Jni(format!("no public {name} folder")));
        }
        let answer = env.call_method(&dir, jni_str!("getAbsolutePath"), jni_sig!("()Ljava/lang/String;"), &[]);
        let path = caught(env, answer)?.l()?;
        let path = crate::content::string_of(env, path)?.ok_or_else(|| crate::Error::Jni(format!("no public {name} folder")))?;
        Ok(PathBuf::from(path))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_dir_for_images_and_documents() {
        let f = Folders::fallback();
        assert_eq!(f.export_dir(Kind::Image), PathBuf::from("/storage/emulated/0/Pictures/LightCraft"));
        assert_eq!(f.export_dir(Kind::Document), PathBuf::from("/storage/emulated/0/Documents/LightCraft"));
        // a work profile's volume, as JNI names it
        let work = Folders::in_volume(Path::new("/storage/emulated/10"));
        assert_eq!(work.export_dir(Kind::Image), PathBuf::from("/storage/emulated/10/Pictures/LightCraft"));
        assert_eq!(work.export_roots().iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Pictures", "Documents"]);
        assert_eq!(work.export_roots()[1].1, PathBuf::from("/storage/emulated/10/Documents/LightCraft"));
        // Pictures and DCIM take images only; Documents and Download take anything
        assert_eq!(f.media_only(), [PathBuf::from("/storage/emulated/0/Pictures"), PathBuf::from("/storage/emulated/0/DCIM")]);
        let local: Vec<_> = f.local_roots().into_iter().map(|(l, p)| (l, p.to_string_lossy().into_owned())).collect();
        assert_eq!(
            local,
            [
                ("DCIM".to_owned(), "/storage/emulated/0/DCIM".to_owned()),
                ("Pictures".to_owned(), "/storage/emulated/0/Pictures".to_owned()),
                ("Download".to_owned(), "/storage/emulated/0/Download".to_owned())
            ]
        );
    }

    #[test]
    fn export_dir_from_partial_answers() {
        // every call answered: those folders
        let f = Folders::from_answers(
            Some("/storage/emulated/10".into()),
            Some("/storage/emulated/10/Pictures".into()),
            Some("/storage/emulated/10/Documents".into()),
            Some("/storage/emulated/10/DCIM".into()),
            Some("/storage/emulated/10/Download".into()),
        );
        assert_eq!(f, Folders::in_volume(Path::new("/storage/emulated/10")));
        // a failed folder call: its place in the volume; a relative answer counts as none
        let f = Folders::from_answers(Some("/storage/emulated/10".into()), None, Some("Documents".into()), None, None);
        assert_eq!(f, Folders::in_volume(Path::new("/storage/emulated/10")));
        // nothing answered: the first user's volume
        assert_eq!(Folders::from_answers(None, None, None, None, None), Folders::fallback());
    }
}
