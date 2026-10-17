//! The debug control server on a device: off unless `debug.lightcraft.control=<port>` is set, and
//! then only for a client that holds the token.
//!
//! Every app on a device shares loopback, so a port alone would let any of them drive LightCraft
//! (import, export, delete). The system property can only be set from a shell, and the token is
//! a random 128-bit value written at each start to `files/control-token` in the app's private
//! storage, which a shell reads with `adb shell run-as <id> cat files/control-token`. The server
//! itself is shared with the desktop (`lightcraft_ui_egui::host::control_server`); this module is
//! the Android start decision and the token file. A token that cannot be made or written means no
//! server, never an open one.

use std::path::{Path, PathBuf};

/// The token file's name inside the app's internal storage folder (`files/`).
pub const TOKEN_FILE: &str = "control-token";

/// Where the token is written for the internal storage folder `files`.
pub fn token_path(files: &Path) -> PathBuf {
    files.join(TOKEN_FILE)
}

/// What the server starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub port: u16,
    pub token: String,
}

/// The token as 32 lower-case hex digits.
pub fn token_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `token` to `path`, readable by the app alone, replacing an earlier one.
pub fn write_token(path: &Path, token: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(token.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// The server to start, if any. `port` is `debug.lightcraft.control`; without it nothing starts and
/// a token left by an earlier start is deleted. With it, `random` supplies 16 random bytes, the
/// token is written to `token_path`, and only then is the server returned. A failure is logged
/// and means no server.
pub fn prepare(port: Option<u16>, token_path: &Path, random: impl FnOnce() -> Result<[u8; 16], String>) -> Option<Server> {
    let Some(port) = port else {
        if let Err(e) = std::fs::remove_file(token_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove the old control token {}: {e}", token_path.display());
        }
        return None;
    };
    let token = match random() {
        Ok(bytes) => token_hex(&bytes),
        Err(e) => {
            log::error!("no control server: no random token ({e})");
            return None;
        }
    };
    if let Err(e) = write_token(token_path, &token) {
        log::error!("no control server: could not write the token ({e})");
        return None;
    }
    log::info!("control server on port {port}; token in {}", token_path.display());
    Some(Server { port, token })
}

/// 16 random bytes from the system.
#[cfg(target_os = "android")]
pub fn random_bytes() -> Result<[u8; 16], String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-android-control-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn no_server_without_property() {
        let dir = temp("none");
        let path = token_path(&dir);
        assert_eq!(prepare(None, &path, || panic!("no token is made without a port")), None);
        assert!(!path.exists());
        // a token left by an earlier start with the property set does not outlive it
        std::fs::write(&path, "old").unwrap();
        assert_eq!(prepare(None, &path, || Ok([0; 16])), None);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_token_is_written_before_the_server_starts() {
        let dir = temp("token");
        let path = token_path(&dir);
        assert_eq!(path, dir.join("control-token"));
        let bytes = [0x00, 0x01, 0xab, 0xff, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0];
        let server = prepare(Some(18555), &path, || Ok(bytes)).unwrap();
        assert_eq!(server.port, 18555);
        assert_eq!(server.token, "0001abff102030405060708090a0b0c0");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), server.token);
        // the next start replaces it, even a longer one
        std::fs::write(&path, "x".repeat(100)).unwrap();
        let again = prepare(Some(18555), &path, || Ok([7; 16])).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), again.token);
        assert_eq!(again.token.len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0, "private to the app");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_token_means_no_server() {
        let dir = temp("fail");
        let path = token_path(&dir);
        assert_eq!(prepare(Some(18555), &path, || Err("entropy unavailable".into())), None);
        assert!(!path.exists());
        // a token that cannot be written (the folder is not there)
        assert_eq!(prepare(Some(18555), &dir.join("missing").join("control-token"), || Ok([1; 16])), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
