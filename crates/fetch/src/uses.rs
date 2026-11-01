//! Every way LightCraft uses the network, in one list. Nothing else may: `tests/uses_scan.rs` fails when source
//! code reaches the network without naming one of these ids, and [`crate::download`] refuses an id that is not here.
//! The list is what Settings ▸ Network shows and what the store's Data safety answers (docs/android.md ▸ Privacy)
//! are written from. LightCraft has no telemetry, no update check and no account: each use below starts with an
//! action of the user's.

use std::sync::atomic::{AtomicBool, Ordering};

/// One use of the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkUse {
    /// The id callers pass (`download*`) or name in a `// network-use: <id>` comment (other sockets).
    pub id: &'static str,
    /// What it is for, in the words Settings shows.
    pub purpose: &'static str,
    /// The hosts it contacts (empty: the user names the other computer).
    pub hosts: &'static [&'static str],
    /// The user action that starts it.
    pub action: &'static str,
    /// Whether "Never use the network" stops it. Off for sockets that stay on this device, and for
    /// `mcp-connect`, a local control connection the user starts by hand.
    pub switchable: bool,
}

pub const DENOISE_MODEL: &str = "denoise-model";
pub const FACE_MODEL: &str = "face-model";
pub const SAM3_MODEL: &str = "sam3-model";
pub const PLACE_NAMES: &str = "place-names";
pub const REMOTE_DENOISE: &str = "remote-denoise";
pub const CONTROL_SERVER: &str = "control-server";
pub const SEGMENT_WORKER: &str = "segment-worker";
pub const MCP_CONNECT: &str = "mcp-connect";

/// All network uses.
pub const USES: &[NetworkUse] = &[
    NetworkUse {
        id: DENOISE_MODEL,
        purpose: "Download the AI Denoise model",
        hosts: &["github.com", "objects.githubusercontent.com"],
        action: "Settings ▸ AI Denoise ▸ Download",
        switchable: true,
    },
    NetworkUse {
        id: FACE_MODEL,
        purpose: "Download a face detection or recognition model",
        hosts: &["github.com", "raw.githubusercontent.com", "huggingface.co", "cdn-lfs.huggingface.co"],
        action: "Settings ▸ Faces ▸ Download",
        switchable: true,
    },
    NetworkUse {
        id: SAM3_MODEL,
        purpose: "Download the SAM 3 mask model from mirrors you list",
        hosts: &[],
        action: "Masking ▸ Download the model",
        switchable: true,
    },
    NetworkUse {
        id: PLACE_NAMES,
        purpose: "Download place names for the map",
        hosts: &["download.geonames.org"],
        action: "Map ▸ Download place names",
        switchable: true,
    },
    NetworkUse {
        id: REMOTE_DENOISE,
        purpose: "Send denoise tiles to a computer you name, on your own network",
        hosts: &[],
        action: "Settings ▸ AI Denoise ▸ Remote computer",
        switchable: true,
    },
    NetworkUse {
        id: CONTROL_SERVER,
        purpose: "Listen for control commands on this device only (loopback; on Android only in debug builds, with a token)",
        hosts: &["127.0.0.1"],
        action: "Starting the app with --control",
        switchable: false,
    },
    NetworkUse {
        id: SEGMENT_WORKER,
        purpose: "Talk to a SAM 3 worker on this device (loopback only)",
        hosts: &["127.0.0.1"],
        action: "Masking with a remote SAM endpoint set",
        switchable: false,
    },
    NetworkUse {
        id: MCP_CONNECT,
        purpose: "Let `lightcraft-cli mcp --connect` drive the running app, at the address you give",
        hosts: &[],
        action: "Running lightcraft-cli mcp --connect",
        switchable: false,
    },
];

/// The use with `id`.
pub fn find(id: &str) -> Option<&'static NetworkUse> {
    USES.iter().find(|u| u.id == id)
}

static NETWORK_OFF: AtomicBool = AtomicBool::new(false);

/// Turn "Never use the network" on or off (process-wide).
pub fn set_network_off(off: bool) {
    NETWORK_OFF.store(off, Ordering::SeqCst);
}

thread_local! {
    static THREAD_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Switch the network off for the calling thread only: lets tests check the refusal without racing the
/// process-wide switch. Not for product code.
#[doc(hidden)]
pub fn set_network_off_on_this_thread(off: bool) {
    THREAD_OFF.with(|c| c.set(off));
}

/// Whether network use is off.
pub fn network_off() -> bool {
    NETWORK_OFF.load(Ordering::SeqCst) || THREAD_OFF.with(std::cell::Cell::get)
}

/// What the user sees when a download is refused by the switch.
pub const NETWORK_OFF_MESSAGE: &str = "Network use is off in Settings ▸ Network";

/// Whether `id` may use the network now.
pub fn check(id: &str) -> Result<&'static NetworkUse, crate::DownloadError> {
    let Some(u) = find(id) else { return Err(crate::DownloadError::UnknownUse(id.to_string())) };
    if u.switchable && network_off() {
        return Err(crate::DownloadError::NetworkOff);
    }
    Ok(u)
}
