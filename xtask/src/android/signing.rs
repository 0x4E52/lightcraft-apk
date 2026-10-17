//! APK signing keys. One concern: where the key lives and the arguments of the tools that create
//! and use it. The debug key is Android's well-known one (`~/.android/debug.keystore`, password
//! `android`); it is created here when missing because the SDK-tools route runs no Gradle/AGP,
//! so a fresh machine has none. It never lives inside the repository. Release signing (APK-37)
//! is added to this module; `apk.rs` and the bundle step share it.

use std::path::{Path, PathBuf};

/// Password of the debug keystore and its key. Public by design (it is Android's convention) and
/// used for debug builds only.
const DEBUG_PASSWORD: &str = "android";

/// Alias of the debug key.
const DEBUG_ALIAS: &str = "androiddebugkey";

/// `~/.android/debug.keystore` for the home folder `home`.
pub fn debug_keystore_path(home: &Path) -> PathBuf {
    home.join(".android").join("debug.keystore")
}

/// The user's home folder from the environment (`HOME`, `USERPROFILE` on Windows).
pub fn home_dir(env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    ["HOME", "USERPROFILE"]
        .iter()
        .find_map(|n| env(n).filter(|v| !v.trim().is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "cannot find the home folder for ~/.android/debug.keystore: set HOME".to_owned())
}

/// The arguments of `keytool` that create the debug keystore at `path`.
pub fn debug_keystore_args(path: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-genkeypair", "-keystore"].iter().map(|s| (*s).to_owned()).collect();
    args.push(path.to_string_lossy().into_owned());
    for a in [
        "-storepass",
        DEBUG_PASSWORD,
        "-keypass",
        DEBUG_PASSWORD,
        "-alias",
        DEBUG_ALIAS,
        "-dname",
        "CN=Android Debug,O=Android,C=US",
        "-keyalg",
        "RSA",
        "-keysize",
        "2048",
        "-validity",
        "10000",
    ] {
        args.push(a.to_owned());
    }
    args
}

/// Makes sure the debug keystore exists: when `path` is missing, creates its folder and calls
/// `create` with the `keytool` arguments. An existing keystore is never touched.
pub fn ensure_debug_keystore(path: &Path, create: impl FnOnce(&[String]) -> Result<(), String>) -> Result<(), String> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    create(&debug_keystore_args(path))?;
    if path.is_file() { Ok(()) } else { Err(format!("keytool did not create {}", path.display())) }
}

/// The `apksigner sign` arguments that sign `unsigned` with the debug key into `signed`.
pub fn debug_sign_args(keystore: &Path, unsigned: &Path, signed: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["sign", "--ks"].iter().map(|s| (*s).to_owned()).collect();
    args.push(keystore.to_string_lossy().into_owned());
    args.push("--ks-pass".to_owned());
    args.push(format!("pass:{DEBUG_PASSWORD}"));
    args.push("--key-pass".to_owned());
    args.push(format!("pass:{DEBUG_PASSWORD}"));
    args.push("--ks-key-alias".to_owned());
    args.push(DEBUG_ALIAS.to_owned());
    args.push("--out".to_owned());
    args.push(signed.to_string_lossy().into_owned());
    args.push(unsigned.to_string_lossy().into_owned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_keystore_created_when_missing_args() {
        let dir = std::env::temp_dir().join(format!("xtask-signing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = debug_keystore_path(&dir);
        assert!(path.ends_with(".android/debug.keystore"), "{path:?}");
        assert!(!path.starts_with(crate::root()), "the key must not live in the repository");

        // missing: keytool runs with the documented arguments (the fake creates the file)
        let mut seen = Vec::new();
        ensure_debug_keystore(&path, |args| {
            seen = args.to_vec();
            std::fs::write(&path, b"key").map_err(|e| e.to_string())
        })
        .unwrap();
        let joined = seen.join(" ");
        assert_eq!(seen.first().map(String::as_str), Some("-genkeypair"));
        for part in [
            format!("-keystore {}", path.display()),
            "-storepass android -keypass android -alias androiddebugkey".to_owned(),
            "-dname CN=Android Debug,O=Android,C=US".to_owned(),
            "-keyalg RSA -keysize 2048 -validity 10000".to_owned(),
        ] {
            assert!(joined.contains(&part), "{part} missing from {joined}");
        }
        // the dname stays one argument
        assert!(seen.iter().any(|a| a == "CN=Android Debug,O=Android,C=US"));

        // present: nothing runs and the file is untouched
        ensure_debug_keystore(&path, |_| Err("must not run".to_owned())).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"key");

        // keytool that fails or creates nothing is an error
        std::fs::remove_file(&path).unwrap();
        assert!(ensure_debug_keystore(&path, |_| Err("boom".to_owned())).is_err());
        assert!(ensure_debug_keystore(&path, |_| Ok(())).unwrap_err().contains("did not create"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn home_comes_from_the_environment() {
        let env = |n: &str| (n == "HOME").then(|| "/home/x".to_owned());
        assert_eq!(home_dir(&env), Ok(PathBuf::from("/home/x")));
        assert!(home_dir(&|_: &str| None).is_err());
        assert!(home_dir(&|_: &str| Some(" ".to_owned())).is_err());
    }

    #[test]
    fn debug_signing_uses_the_debug_key() {
        let args = debug_sign_args(Path::new("/k/debug.keystore"), Path::new("/in.apk"), Path::new("/out.apk"));
        let joined = args.join(" ");
        assert_eq!(args.first().map(String::as_str), Some("sign"));
        assert!(joined.contains("--ks /k/debug.keystore --ks-pass pass:android"), "{joined}");
        assert!(joined.contains("--ks-key-alias androiddebugkey --out /out.apk /in.apk"), "{joined}");
    }
}
