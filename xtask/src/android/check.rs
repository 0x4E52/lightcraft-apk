//! `cargo xtask android check`: clippy for `aarch64-linux-android` over every workspace crate the
//! APK links. On x86_64 the `cfg(target_os = "android")` modules compile empty and are never linted,
//! so the normal clippy step cannot see them.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::StepOutcome;

/// The Rust target the APK is built for.
const TARGET: &str = "aarch64-linux-android";

/// The workspace package that will be the APK's entry crate (APK-10).
const ROOT: &str = "lightcraft-android";

/// Roots used until `ROOT` exists: the two crates already known to compile for Android.
const FALLBACK_ROOTS: [&str; 2] = ["lightcraft-engine", "lightcraft-ui-egui"];

/// Cargo features the Android build enables (never `--workspace`: that pulls `apps/lightcraft`,
/// whose eframe `accesskit` feature does not compile with `android-native-activity`).
const FEATURES: &str = "lightcraft-engine/sam,lightcraft-engine/denoise";

/// The workspace packages the APK links: every one reachable over normal (not dev, not build)
/// dependencies from `lightcraft-android`, or from engine and ui-egui while that crate doesn't exist.
/// Sorted; empty when `metadata` has no packages.
pub fn android_set(metadata: &Value) -> Vec<String> {
    let packages = metadata["packages"].as_array().map(Vec::as_slice).unwrap_or_default();
    let members: BTreeSet<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();
    let roots: Vec<&str> = if members.contains(ROOT) { vec![ROOT] } else { FALLBACK_ROOTS.iter().copied().filter(|r| members.contains(r)).collect() };
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut todo = roots;
    while let Some(name) = todo.pop() {
        if !seen.insert(name) {
            continue;
        }
        let Some(pkg) = packages.iter().find(|p| p["name"].as_str() == Some(name)) else { continue };
        for dep in pkg["dependencies"].as_array().map(Vec::as_slice).unwrap_or_default() {
            // `kind` is null for a normal dependency
            if !matches!(dep["kind"].as_str(), None | Some("normal")) {
                continue;
            }
            if let Some(d) = dep["name"].as_str().filter(|d| members.contains(d)) {
                todo.push(d);
            }
        }
    }
    seen.into_iter().map(str::to_owned).collect()
}

/// Whether `rustup target list --installed` printed `target`.
fn target_installed(rustup_output: &str, target: &str) -> bool {
    rustup_output.lines().any(|l| l.trim() == target)
}

/// The decision, with the outside world injected: `rustup` is the output of `rustup target list
/// --installed` (or why it failed), `lint` runs clippy on the package set.
fn check_with(rustup: Result<String, String>, metadata: &Value, lint: impl FnOnce(&[String]) -> Result<(), String>) -> Result<StepOutcome, String> {
    // No rustup (a distro toolchain) or no target: nothing to lint with, which is not a failure.
    match rustup {
        Ok(out) if target_installed(&out, TARGET) => {}
        Ok(_) => return Ok(StepOutcome::Skipped(format!("{TARGET} not installed"))),
        Err(e) => return Ok(StepOutcome::Skipped(format!("{TARGET} not checked: {e}"))),
    }
    let set = android_set(metadata);
    if set.is_empty() {
        return Err("android check: no workspace package to lint (cargo metadata lists none of the Android crates)".into());
    }
    lint(&set)?;
    Ok(StepOutcome::Ran)
}

fn rustup_installed() -> Result<String, String> {
    let out = std::process::Command::new("rustup").args(["target", "list", "--installed"]).output().map_err(|e| format!("rustup: {e}"))?;
    if !out.status.success() {
        return Err(format!("rustup target list failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn clippy(set: &[String]) -> Result<(), String> {
    let mut c = crate::cargo();
    c.args(["clippy", "--target", TARGET]);
    for p in set {
        c.args(["-p", p]);
    }
    c.args(["--features", FEATURES, "--", "-D", "warnings"]);
    crate::run(
        c,
        &format!(
            "cargo clippy --target {TARGET} {} --features {FEATURES} -- -D warnings",
            set.iter().map(|p| format!("-p {p}")).collect::<Vec<_>>().join(" ")
        ),
    )
}

/// Lint the Android crates (`cargo xtask android check`, and a `ci` step).
pub fn check() -> Result<StepOutcome, String> {
    check_with(rustup_installed(), &crate::metadata()?, clippy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pkg(name: &str, deps: &[(&str, Option<&str>)]) -> Value {
        json!({"name": name, "dependencies": deps.iter().map(|(n, k)| json!({"name": n, "kind": k})).collect::<Vec<_>>()})
    }

    #[test]
    fn android_set_follows_normal_deps() {
        let meta = json!({"packages": [
            pkg("lightcraft-android", &[("lightcraft-ui-egui", None), ("lightcraft-android-sys", None), ("jni", None), ("lightcraft-testkit", Some("dev"))]),
            pkg("lightcraft-android-sys", &[("jni", None)]),
            pkg("lightcraft-ui-egui", &[("lightcraft-engine", None), ("egui", None)]),
            pkg("lightcraft-engine", &[("lightcraft-geom", None), ("lightcraft-gpu", Some("normal")), ("lightcraft-build-helper", Some("build"))]),
            pkg("lightcraft-geom", &[]),
            pkg("lightcraft-gpu", &[("lightcraft-geom", None)]),
            pkg("lightcraft-build-helper", &[]),
            pkg("lightcraft-testkit", &[("lightcraft-geom", None)]),
            pkg("lightcraft", &[("lightcraft-ui-egui", None)]),
        ]});
        assert_eq!(
            android_set(&meta),
            ["lightcraft-android", "lightcraft-android-sys", "lightcraft-engine", "lightcraft-geom", "lightcraft-gpu", "lightcraft-ui-egui"],
            "dev and build dependencies and the desktop app stay out"
        );
    }

    #[test]
    fn android_set_falls_back_to_engine_and_ui() {
        let meta = json!({"packages": [
            pkg("lightcraft-ui-egui", &[("lightcraft-engine", None), ("lightcraft-testkit", Some("dev"))]),
            pkg("lightcraft-engine", &[("lightcraft-geom", None), ("serde", None)]),
            pkg("lightcraft-geom", &[]),
            pkg("lightcraft-testkit", &[]),
            pkg("lightcraft-cli", &[("lightcraft-ui-egui", None)]),
        ]});
        assert_eq!(android_set(&meta), ["lightcraft-engine", "lightcraft-geom", "lightcraft-ui-egui"]);
        assert!(android_set(&json!({})).is_empty());
    }

    #[test]
    fn missing_target_is_skipped() {
        let meta = json!({"packages": [pkg("lightcraft-engine", &[]), pkg("lightcraft-ui-egui", &[])]});
        let never = |_: &[String]| -> Result<(), String> { Err("clippy must not run without the target".into()) };
        let installed = "wasm32-unknown-unknown\nx86_64-unknown-linux-gnu\n".to_owned();
        assert_eq!(check_with(Ok(installed), &meta, never), Ok(StepOutcome::Skipped("aarch64-linux-android not installed".into())));
        // a prefix of the name is not the target
        let similar = "aarch64-linux-android-foo\n".to_owned();
        assert!(matches!(check_with(Ok(similar), &meta, never), Ok(StepOutcome::Skipped(_))));
        assert!(matches!(check_with(Err("rustup: not found".into()), &meta, never), Ok(StepOutcome::Skipped(why)) if why.contains("rustup")));
    }

    #[test]
    fn installed_target_lints_the_set_and_reports_failures() {
        let meta = json!({"packages": [pkg("lightcraft-engine", &[]), pkg("lightcraft-ui-egui", &[])]});
        let installed = "aarch64-linux-android\nx86_64-unknown-linux-gnu\n".to_owned();
        let seen = std::cell::RefCell::new(Vec::new());
        let ok = check_with(Ok(installed.clone()), &meta, |set| {
            seen.borrow_mut().extend_from_slice(set);
            Ok(())
        });
        assert_eq!(ok, Ok(StepOutcome::Ran));
        assert_eq!(*seen.borrow(), ["lightcraft-engine", "lightcraft-ui-egui"]);
        assert_eq!(check_with(Ok(installed.clone()), &meta, |_| Err("lint failed".into())), Err("lint failed".into()));
        assert!(check_with(Ok(installed), &json!({"packages": []}), |_| Ok(())).is_err(), "an empty set is an error, not a pass");
    }
}
