//! The Android manifest: `packaging/android/AndroidManifest.xml` is a template, this module fills
//! its `${...}` values and derives the version code (docs/android-port.md, APK-13).

use std::path::{Path, PathBuf};

use super::sdk::{MIN_SDK, TARGET_SDK};

/// The application id of a release build (decision 1 in docs/android-port.md). Never the
/// original project's id.
pub const APPLICATION_ID: &str = "io.github.n0x4e52.lightcraft";

/// The launcher label (decision 1): a literal string, so the manifest needs no resource.
pub const APP_LABEL: &str = "LightCraft";

/// The highest version code Google Play accepts.
const MAX_VERSION_CODE: u64 = 2_100_000_000;

/// How many numbered builds of each pre-release stage have a slot below the next stage.
const STAGE_SLOTS: u32 = 3;

/// Debug and release builds install side by side: the debug id has a suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Debug,
    Release,
}

impl Build {
    pub fn application_id(self) -> String {
        match self {
            Build::Debug => format!("{APPLICATION_ID}.debug"),
            Build::Release => APPLICATION_ID.to_owned(),
        }
    }
}

/// The Android `versionCode` for a workspace version (`xtask/src/version.rs`):
/// `major*1_000_000 + minor*1_000 + patch*10 + stage`, with stage 9 for a release, `alpha.N` → N-1,
/// `beta.N` → 2+N and `rc.N` → 5+N for N in 1..=3, so alpha < beta < rc < release. Anything that
/// would collide or that this scheme has no slot for is an error.
pub fn version_code(version: &str) -> Result<u32, String> {
    crate::version::validate(version)?;
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (version, None),
    };
    let mut nums = core.split('.').map(|n| n.parse::<u64>().map_err(|e| format!("`{version}`: {e}")));
    let (Some(major), Some(minor), Some(patch)) = (nums.next(), nums.next(), nums.next()) else {
        return Err(format!("`{version}` is not MAJOR.MINOR.PATCH"));
    };
    let (major, minor, patch) = (major?, minor?, patch?);
    if minor > 999 {
        return Err(format!("`{version}`: minor {minor} is over 999 and would collide in the version code"));
    }
    if patch > 99 {
        return Err(format!("`{version}`: patch {patch} is over 99 and would collide in the version code"));
    }
    let stage = match pre {
        None => 9,
        Some(tag) => pre_release_stage(version, tag)?,
    };
    let code = major * 1_000_000 + minor * 1_000 + patch * 10 + u64::from(stage);
    if code > MAX_VERSION_CODE {
        return Err(format!("`{version}`: version code {code} is over Google Play's limit {MAX_VERSION_CODE}"));
    }
    u32::try_from(code).map_err(|_| format!("`{version}`: version code {code} does not fit"))
}

/// The stage digit (0 to 8) of `alpha.N`, `beta.N` or `rc.N`.
fn pre_release_stage(version: &str, tag: &str) -> Result<u32, String> {
    let unsupported = || format!("`{version}`: pre-release `{tag}` has no version code; use alpha.N, beta.N or rc.N with N in 1..={STAGE_SLOTS}");
    let (name, n) = tag.split_once('.').ok_or_else(unsupported)?;
    let n: u32 = n.parse().map_err(|_| unsupported())?;
    // `+05` and `rc.01` are different tags that would give the same code.
    if n.to_string() != tag.rsplit('.').next().unwrap_or_default() || !(1..=STAGE_SLOTS).contains(&n) {
        return Err(unsupported());
    }
    match name {
        "alpha" => Ok(n - 1),
        "beta" => Ok(2 + n),
        "rc" => Ok(5 + n),
        _ => Err(unsupported()),
    }
}

/// `text` as an XML attribute value.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// `template` with every `${...}` value filled for `version` and `build`.
pub fn fill(template: &str, version: &str, build: Build) -> Result<String, String> {
    let code = version_code(version)?;
    let values = [
        ("APPLICATION_ID", escape(&build.application_id())),
        ("APP_LABEL", escape(APP_LABEL)),
        ("VERSION_CODE", code.to_string()),
        ("VERSION_NAME", escape(version)),
        ("MIN_SDK", MIN_SDK.to_string()),
        ("TARGET_SDK", TARGET_SDK.to_string()),
    ];
    let mut out = template.to_owned();
    for (key, value) in &values {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    if let Some(at) = out.find("${") {
        let rest = out.get(at..).unwrap_or_default();
        let name: String = rest.chars().take_while(|c| *c != '}' && *c != '\n').collect();
        return Err(format!("AndroidManifest.xml: unknown placeholder `{name}}}`"));
    }
    Ok(out)
}

/// Fills `packaging/android/AndroidManifest.xml` for the workspace version and writes it to
/// `<dir>/AndroidManifest.xml`; returns that path.
pub fn write(build: Build, dir: &Path) -> Result<PathBuf, String> {
    let root = crate::root();
    let read = |path: &Path| std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()));
    let template = read(&root.join("packaging/android/AndroidManifest.xml"))?;
    let version = crate::version::read(&read(&root.join("Cargo.toml"))?)?;
    let filled = fill(&template, &version, build)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let out = dir.join("AndroidManifest.xml");
    std::fs::write(&out, filled).map_err(|e| format!("write {}: {e}", out.display()))?;
    println!("android manifest: {} ({}, version {version}, code {})", out.display(), build.application_id(), version_code(&version)?);
    Ok(out)
}

/// `cargo xtask android manifest [--release]`: writes `target/android/AndroidManifest.xml`.
pub fn run(args: &[&str]) -> Result<(), String> {
    let build = match args {
        [] => Build::Debug,
        ["--release"] => Build::Release,
        other => return Err(format!("android manifest: unexpected arguments {other:?} (only --release)")),
    };
    write(build, &crate::root().join("target").join("android")).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The elements of `xml` (comments dropped) as `(name, attributes)`.
    fn elements(xml: &str) -> Vec<(String, Vec<(String, String)>)> {
        let mut text = String::new();
        let mut rest = xml;
        while let Some(start) = rest.find("<!--") {
            text.push_str(&rest[..start]);
            let end = rest[start..].find("-->").expect("comment is closed");
            rest = &rest[start + end + 3..];
        }
        text.push_str(rest);
        let mut out = Vec::new();
        for tag in text.split('<').skip(1) {
            let tag = tag.split('>').next().unwrap();
            if tag.starts_with('?') || tag.starts_with('/') {
                continue;
            }
            let tag = tag.trim_end_matches('/');
            let name: String = tag.chars().take_while(|c| !c.is_whitespace()).collect();
            let mut attrs = Vec::new();
            let mut s = &tag[name.len()..];
            while let Some((key, after)) = s.split_once("=\"") {
                let (value, next) = after.split_once('"').expect("attribute value is closed");
                attrs.push((key.trim().to_owned(), value.to_owned()));
                s = next;
            }
            out.push((name, attrs));
        }
        out
    }

    fn attr(xml: &str, element: &str, name: &str) -> String {
        let all = elements(xml);
        let (_, attrs) = all.iter().find(|(e, _)| e == element).unwrap_or_else(|| panic!("no <{element}>"));
        attrs.iter().find(|(k, _)| k == name).unwrap_or_else(|| panic!("<{element}> has no {name}")).1.clone()
    }

    fn template() -> String {
        std::fs::read_to_string(crate::root().join("packaging/android/AndroidManifest.xml")).unwrap()
    }

    #[test]
    fn manifest_has_required_keys() {
        let xml = fill(&template(), "1.2.3-rc.2", Build::Release).unwrap();
        assert!(!xml.contains("${"), "unfilled placeholder");
        let a = |el: &str, name: &str| attr(&xml, el, &format!("android:{name}"));
        assert_eq!(attr(&xml, "manifest", "package"), "io.github.n0x4e52.lightcraft");
        assert_eq!(a("manifest", "versionCode"), "1002037");
        assert_eq!(a("manifest", "versionName"), "1.2.3-rc.2");
        assert_eq!(a("uses-sdk", "minSdkVersion"), "30");
        assert_eq!(a("uses-sdk", "targetSdkVersion"), "36");
        assert_eq!(a("application", "label"), "LightCraft");
        assert_eq!(a("application", "icon"), "@mipmap/ic_launcher");
        assert_eq!(a("application", "roundIcon"), "@mipmap/ic_launcher");
        assert_eq!(a("application", "hasCode"), "false");
        assert_eq!(a("application", "allowBackup"), "false");
        assert_eq!(a("application", "extractNativeLibs"), "false");
        assert_eq!(a("application", "enableOnBackInvokedCallback"), "false");
        assert_eq!(a("activity", "name"), "android.app.NativeActivity");
        assert_eq!(a("activity", "exported"), "true");
        assert_eq!(a("activity", "launchMode"), "singleTask");
        assert_eq!(a("activity", "theme"), "@android:style/Theme.DeviceDefault.NoActionBar");
        let changes = a("activity", "configChanges");
        let have: Vec<&str> = changes.split('|').collect();
        for want in [
            "mcc",
            "mnc",
            "locale",
            "touchscreen",
            "keyboard",
            "keyboardHidden",
            "navigation",
            "screenLayout",
            "fontScale",
            "uiMode",
            "orientation",
            "density",
            "screenSize",
            "smallestScreenSize",
            "layoutDirection",
            "colorMode",
            "fontWeightAdjustment",
            "grammaticalGender",
        ] {
            assert!(have.contains(&want), "configChanges lacks {want}");
        }
        assert_eq!(have.len(), 18, "configChanges has extra values: {changes}");
        assert_eq!(a("meta-data", "name"), "android.app.lib_name");
        assert_eq!(a("meta-data", "value"), "lightcraft_android");
        assert_eq!(a("action", "name"), "android.intent.action.MAIN");
        assert_eq!(a("category", "name"), "android.intent.category.LAUNCHER");
    }

    #[test]
    fn manifest_requests_internet() {
        // model downloads and the debug control server's loopback socket need it
        let xml = fill(&template(), "1.2.3", Build::Debug).unwrap();
        let permissions: Vec<String> = elements(&xml)
            .into_iter()
            .filter(|(e, _)| e == "uses-permission")
            .filter_map(|(_, attrs)| attrs.into_iter().find(|(k, _)| k == "android:name"))
            .map(|(_, v)| v)
            .collect();
        assert!(permissions.iter().any(|p| p == "android.permission.INTERNET"), "{permissions:?}");
    }

    #[test]
    fn debug_build_uses_debug_suffix() {
        let debug = fill(&template(), "0.1.0", Build::Debug).unwrap();
        assert_eq!(attr(&debug, "manifest", "package"), "io.github.n0x4e52.lightcraft.debug");
        let release = fill(&template(), "0.1.0", Build::Release).unwrap();
        assert_eq!(attr(&release, "manifest", "package"), "io.github.n0x4e52.lightcraft");
        assert!(!debug.contains("ai.storyteller"));
    }

    #[test]
    fn unknown_placeholder_is_error() {
        let err = fill("<a x=\"${NOPE}\"/>", "1.0.0", Build::Debug).unwrap_err();
        assert!(err.contains("NOPE"), "{err}");
    }

    #[test]
    fn label_is_escaped() {
        assert_eq!(escape("A&B \"C\" <d>"), "A&amp;B &quot;C&quot; &lt;d&gt;");
    }

    #[test]
    fn version_code_is_monotonic() {
        let versions = ["0.1.0", "0.1.1", "0.1.9", "0.2.0", "0.999.99", "1.0.0", "1.0.1", "1.1.0", "2.0.0"];
        let codes: Vec<u32> = versions.iter().map(|v| version_code(v).unwrap()).collect();
        assert_eq!(codes[0], 1009);
        assert_eq!(version_code("1.2.3").unwrap(), 1_002_039);
        assert!(codes.windows(2).all(|w| w[0] < w[1]), "{codes:?}");
    }

    #[test]
    fn prerelease_sorts_before_release() {
        for pre in ["alpha.1", "alpha.3", "beta.1", "beta.3", "rc.1", "rc.3"] {
            let v = format!("1.2.3-{pre}");
            assert!(version_code(&v).unwrap() < version_code("1.2.3").unwrap(), "{v}");
            assert!(version_code(&v).unwrap() > version_code("1.2.2").unwrap(), "{v}");
        }
    }

    #[test]
    fn alpha_before_beta_before_rc() {
        let order = ["alpha.1", "alpha.2", "alpha.3", "beta.1", "beta.2", "beta.3", "rc.1", "rc.2", "rc.3"];
        let codes: Vec<u32> = order.iter().map(|p| version_code(&format!("1.0.0-{p}")).unwrap()).collect();
        assert!(codes.windows(2).all(|w| w[0] < w[1]), "{codes:?}");
        assert_eq!(codes[0], 1_000_000);
        assert_eq!(codes[6], 1_000_006);
    }

    #[test]
    fn unsupported_prerelease_is_error() {
        for v in ["1.0.0-alpha", "1.0.0-rc.12", "1.0.0-rc.4", "1.0.0-rc.0", "1.0.0-dev.1", "1.0.0-rc.1.2", "1.0.0-rc.01", "1.0.0-x-y.2"] {
            assert!(version_code(v).is_err(), "{v}");
        }
        for v in ["1.1000.0", "1.0.100", "2100.0.0", "1.0", "nope"] {
            assert!(version_code(v).is_err(), "{v}");
        }
    }

    #[test]
    fn the_workspace_version_has_a_code() {
        let text = std::fs::read_to_string(crate::root().join("Cargo.toml")).unwrap();
        version_code(&crate::version::read(&text).unwrap()).unwrap();
    }
}
