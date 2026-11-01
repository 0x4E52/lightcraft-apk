//! No source reaches the network without naming a registered use (`lightcraft_fetch::uses`). Scans the
//! workspace's non-test Rust sources for downloads, sockets and HTTP clients; each hit must carry a
//! `// network-use: <id>` comment (on its line or one of the three before it) with an id of the registry.

use std::path::{Path, PathBuf};

use lightcraft_fetch::uses::{USES, find};

/// What reaches the network (or listens for it).
const PATTERNS: &[&str] = &[
    "lightcraft_fetch::download",
    "fetch::download",
    "fetch::http::get",
    "http::get(",
    "TcpStream::connect",
    "TcpListener::bind",
    "UdpSocket",
    "ureq::",
    "reqwest::",
    "hyper::",
    "attohttpc::",
    "minreq::",
    "surf::",
    "isahc::",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !matches!(name.as_str(), "target" | "tests" | "benches" | "examples" | ".git") {
                rust_files(&p, out);
            }
        } else if name.ends_with(".rs") && !name.starts_with("tests") && !name.ends_with("_tests.rs") {
            out.push(p);
        }
    }
}

/// The non-test part of `text`: the lines of `#[cfg(test…)]` items are blanked (a `mod x;` is one line, a
/// `mod x { … }` runs to its closing brace), so line numbers and everything after the item are kept.
fn production(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut guard = false;
    let mut depth: i32 = 0;
    let mut in_item = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        let skip = if in_item {
            true
        } else if t.starts_with("#[cfg(test") || t.starts_with("#[cfg(all(test") {
            guard = true;
            true
        } else if guard && t.starts_with("#[") {
            true
        } else if guard {
            guard = false;
            in_item = true;
            depth = 0;
            true
        } else {
            false
        };
        if skip {
            if in_item {
                depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
                // a declaration ends at `;` before any brace, or at the brace that closes its body
                if (depth <= 0 && line.contains('}')) || (depth == 0 && t.ends_with(';')) {
                    in_item = false;
                }
            }
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Offending `file:line` entries of `text` (an unregistered or unmarked network call).
fn offenders(name: &str, text: &str) -> Vec<String> {
    let prod = production(text);
    let lines: Vec<&str> = prod.lines().collect();
    let mut bad = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") || !PATTERNS.iter().any(|p| line.contains(p)) {
            continue;
        }
        let from = i.saturating_sub(3);
        let marked = lines
            .get(from..=i)
            .unwrap_or(&[])
            .iter()
            .any(|l| l.split_once("network-use:").is_some_and(|(_, id)| find(id.trim().trim_end_matches('.')).is_some()));
        if !marked {
            bad.push(format!("{name}:{}: {}", i + 1, code));
        }
    }
    bad
}

#[test]
fn every_network_call_is_registered() {
    let mut files = Vec::new();
    for d in ["crates", "apps"] {
        rust_files(&workspace().join(d), &mut files);
    }
    assert!(files.len() > 100, "the scan found the sources ({})", files.len());
    let mut bad = Vec::new();
    for f in &files {
        // the registry and this crate's own transport are the definition, not a use
        let rel = f.strip_prefix(workspace()).unwrap_or(f).to_string_lossy().replace('\\', "/");
        if rel.starts_with("crates/fetch/") || rel.contains("/xtask/") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(f) {
            bad.extend(offenders(&rel, &text));
        }
    }
    assert!(bad.is_empty(), "network use without a `// network-use: <id>` from lightcraft_fetch::uses:\n{}", bad.join("\n"));
}

#[test]
fn the_scan_catches_an_unmarked_call() {
    let bad = offenders("x.rs", "fn f() {\n    let s = std::net::TcpStream::connect(a);\n}\n");
    assert_eq!(bad.len(), 1);
    let ok = offenders("x.rs", "fn f() {\n    // network-use: remote-denoise\n    let s = std::net::TcpStream::connect(a);\n}\n");
    assert!(ok.is_empty());
    let unknown = offenders("x.rs", "// network-use: telemetry\nlet s = TcpStream::connect(a);\n");
    assert_eq!(unknown.len(), 1);
    let in_tests = offenders("x.rs", "fn f() {}\n#[cfg(test)]\nmod t { fn g() { TcpStream::connect(a); } }\n");
    assert!(in_tests.is_empty());
    // a one-line `#[cfg(test)] mod t;` hides only itself: what follows is scanned
    let after = offenders("x.rs", "#[cfg(test)]\nmod t;\nfn f() {\n    TcpStream::connect(a);\n}\n");
    assert_eq!(after.len(), 1);
    let after_inline = offenders("x.rs", "#[cfg(test)]\nmod t { fn g() { TcpStream::connect(a); } }\nfn f() { TcpStream::connect(a); }\n");
    assert_eq!(after_inline.len(), 1);
    let after_block =
        offenders("x.rs", "#[cfg(test)]\nmod t {\n    fn g() {\n        TcpStream::connect(a);\n    }\n}\nfn f() {\n    TcpStream::connect(a);\n}\n");
    assert_eq!(after_block.len(), 1);
}

#[test]
fn registry_is_consistent() {
    let mut ids: Vec<&str> = USES.iter().map(|u| u.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), USES.len(), "ids are unique");
    assert!(USES.iter().all(|u| !u.purpose.is_empty() && !u.action.is_empty()));
}
