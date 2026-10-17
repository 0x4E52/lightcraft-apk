//! Where the Android SDK and NDK are: environment variables first, then `<sdk>/ndk/<version>`.
//! The NDK's version comes from its `source.properties`.

use std::path::{Path, PathBuf};

/// The lowest Android API level the app runs on (decision 3 in docs/android-port.md): the clang
/// wrapper that links the `.so` is named after it.
pub const MIN_SDK: u32 = 30;

/// The API level the app targets and the platform it is linked against (decision 3 in
/// docs/android-port.md: Play's rule for new apps and updates since 2026-08-31).
pub const TARGET_SDK: u32 = 36;

/// An NDK revision, `major.minor.patch` (a `-beta1` suffix is dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NdkVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl NdkVersion {
    /// `27.2.12479018` (also `r27`-less forms such as `29.0.13113456-beta1`).
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('.');
        let major = parts.next()?.trim().parse().ok()?;
        let minor = parts.next()?.trim().parse().ok()?;
        let patch_text = parts.next()?;
        let digits: String = patch_text.chars().take_while(char::is_ascii_digit).collect();
        Some(Self { major, minor, patch: digits.parse().ok()? })
    }
}

/// An installed NDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ndk {
    pub root: PathBuf,
    pub version: NdkVersion,
}

/// The version in the text of an NDK's `source.properties` (`Pkg.Revision = 27.2.12479018`).
pub fn parse_ndk_version(source_properties: &str) -> Result<NdkVersion, String> {
    let value = source_properties
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "Pkg.Revision")
        .map(|(_, v)| v.trim())
        .ok_or("source.properties has no Pkg.Revision line")?;
    NdkVersion::parse(value).ok_or_else(|| format!("source.properties: cannot read the NDK revision `{value}`"))
}

/// The first of `names` that `env` sets to a non-empty value.
fn first_set(env: &dyn Fn(&str) -> Option<String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| env(n).filter(|v| !v.trim().is_empty()))
}

/// The SDK folder: `ANDROID_HOME`, else `ANDROID_SDK_ROOT`.
pub fn sdk_root(env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    first_set(env, &["ANDROID_HOME", "ANDROID_SDK_ROOT"])
        .map(PathBuf::from)
        .ok_or_else(|| "the Android SDK is not set up: set ANDROID_HOME (or ANDROID_SDK_ROOT) to its folder".to_owned())
}

/// The newest `<sdk>/ndk/<version>` folder among `names`, by version.
fn newest_ndk_dir(names: &[String]) -> Option<&String> {
    names.iter().filter_map(|n| NdkVersion::parse(n).map(|v| (v, n))).max().map(|(_, n)| n)
}

fn read_ndk(root: PathBuf) -> Result<Ndk, String> {
    let props = root.join("source.properties");
    let text = std::fs::read_to_string(&props).map_err(|e| format!("{}: {e} (is this an NDK folder?)", props.display()))?;
    let version = parse_ndk_version(&text).map_err(|e| format!("{}: {e}", props.display()))?;
    Ok(Ndk { root, version })
}

/// The NDK: `ANDROID_NDK_HOME`, else `ANDROID_NDK_LATEST_HOME`, else the newest folder in
/// `<sdk>/ndk`. The error says what to set.
pub fn ndk(env: &dyn Fn(&str) -> Option<String>) -> Result<Ndk, String> {
    if let Some(dir) = first_set(env, &["ANDROID_NDK_HOME", "ANDROID_NDK_LATEST_HOME"]) {
        return read_ndk(PathBuf::from(dir));
    }
    let missing = || "the Android NDK is not set up: set ANDROID_NDK_HOME (or install it under $ANDROID_HOME/ndk)".to_owned();
    let ndk_dir = sdk_root(env).map_err(|_| missing())?.join("ndk");
    let names: Vec<String> = std::fs::read_dir(&ndk_dir).map_err(|_| missing())?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    let newest = newest_ndk_dir(&names).ok_or_else(missing)?;
    read_ndk(ndk_dir.join(newest))
}

/// The NDK's prebuilt LLVM folder for the machine running the build.
pub fn llvm_bin(ndk_root: &Path) -> PathBuf {
    let host = if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(windows) {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    };
    ndk_root.join("toolchains").join("llvm").join("prebuilt").join(host).join("bin")
}

/// The process environment, for `ndk` / `sdk_root`.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndk_version_parses() {
        let props = "Pkg.Desc = Android NDK\nPkg.Revision = 27.2.12479018\n";
        assert_eq!(parse_ndk_version(props), Ok(NdkVersion { major: 27, minor: 2, patch: 12479018 }));
        // a preview build keeps its numbers
        assert_eq!(parse_ndk_version("Pkg.Revision=29.0.13113456-beta1"), Ok(NdkVersion { major: 29, minor: 0, patch: 13113456 }));
        assert!(parse_ndk_version("Pkg.Desc = Android NDK\n").is_err(), "no revision line");
        assert!(parse_ndk_version("Pkg.Revision = banana").is_err());
        assert!(NdkVersion { major: 28, minor: 0, patch: 0 } > NdkVersion { major: 27, minor: 9, patch: 99 });
    }

    #[test]
    fn missing_sdk_and_ndk_say_what_to_set() {
        let none = |_: &str| None;
        assert!(sdk_root(&none).unwrap_err().contains("ANDROID_HOME"));
        assert!(ndk(&none).unwrap_err().contains("ANDROID_NDK_HOME"));
        let blank = |_: &str| Some("  ".to_owned());
        assert!(sdk_root(&blank).is_err(), "an empty variable counts as unset");
    }

    #[test]
    fn sdk_root_prefers_android_home() {
        let env = |n: &str| match n {
            "ANDROID_HOME" => Some("/a".to_owned()),
            "ANDROID_SDK_ROOT" => Some("/b".to_owned()),
            _ => None,
        };
        assert_eq!(sdk_root(&env), Ok(PathBuf::from("/a")));
    }

    #[test]
    fn ndk_is_read_from_its_folder() {
        let dir = std::env::temp_dir().join(format!("xtask-ndk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.properties"), "Pkg.Revision = 28.1.13356709\n").unwrap();
        let root = dir.to_string_lossy().into_owned();
        let env = |n: &str| (n == "ANDROID_NDK_LATEST_HOME").then(|| root.clone());
        assert_eq!(ndk(&env), Ok(Ndk { root: dir.clone(), version: NdkVersion { major: 28, minor: 1, patch: 13356709 } }));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ndk(&env).unwrap_err().contains("source.properties"));
    }

    #[test]
    fn newest_ndk_folder_wins_by_version() {
        let names: Vec<String> = ["27.2.12479018", "28.0.1", "junk", "9.1.1"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(newest_ndk_dir(&names).map(String::as_str), Some("28.0.1"));
        assert_eq!(newest_ndk_dir(&["junk".to_owned()]), None);
    }
}
