//! `cargo xtask android apk [--abi arm64-v8a|x86_64]…`: package a debug APK from the `.so` files
//! of `cargo xtask android so`, the filled manifest and `packaging/android/res/` with the SDK
//! build-tools (decision 6, route a: no Gradle). The steps are `aapt2 compile` → `aapt2 link` →
//! append the `.so` Stored and 16 KB-aligned (zip_lib.rs) → `zipalign` → `apksigner` → `zipalign -c`.
//! Needs the SDK (platform and build-tools, sdk.rs) and a JDK for `keytool`; the argument builders
//! are pure and unit-tested.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::manifest::{self, Build};
use super::sdk::{self, MIN_SDK, NdkVersion, TARGET_SDK};
use super::signing;
use super::so::{self, Abi, LIB_FILE};
use super::zip_lib::{self, NativeLib};

/// build-tools matching the target platform (decision 3: build-tools >= 36).
const MIN_BUILD_TOOLS: u32 = TARGET_SDK;

/// The platform the app is linked against: `<sdk>/platforms/android-36/android.jar`.
pub fn android_jar(sdk_root: &Path) -> PathBuf {
    sdk_root.join("platforms").join(format!("android-{TARGET_SDK}")).join("android.jar")
}

/// The newest `<sdk>/build-tools/<version>` folder among `names` that is at least [`MIN_BUILD_TOOLS`].
/// A stable release (no `-rc1` style suffix) wins over any pre-release; a pre-release is used only
/// when no stable one qualifies.
fn newest_build_tools(names: &[String]) -> Option<&String> {
    let candidates = || names.iter().filter_map(|n| NdkVersion::parse(n).map(|v| (v, n))).filter(|(v, _)| v.major >= MIN_BUILD_TOOLS);
    candidates().filter(|(_, n)| !n.contains('-')).max().or_else(|| candidates().max()).map(|(_, n)| n)
}

/// The build-tools folder to use.
fn build_tools_dir(sdk_root: &Path) -> Result<PathBuf, String> {
    let dir = sdk_root.join("build-tools");
    let missing = || {
        format!(
            "no SDK build-tools {MIN_BUILD_TOOLS} or newer in {} (sdkmanager \"build-tools;{MIN_BUILD_TOOLS}.0.0\" \"platforms;android-{TARGET_SDK}\")",
            dir.display()
        )
    };
    let names: Vec<String> = std::fs::read_dir(&dir).map_err(|_| missing())?.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    Ok(dir.join(newest_build_tools(&names).ok_or_else(missing)?))
}

/// `name` inside the build-tools folder, with the platform's extension.
fn tool(build_tools: &Path, name: &str) -> PathBuf {
    let ext = match (cfg!(windows), name) {
        (true, "apksigner") => ".bat",
        (true, _) => ".exe",
        (false, _) => "",
    };
    build_tools.join(format!("{name}{ext}"))
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

/// `aapt2 compile --dir <res> -o <compiled.zip>`.
pub fn aapt2_compile_args(res_dir: &Path, compiled: &Path) -> Vec<String> {
    let mut args = strings(&["compile", "--dir"]);
    args.push(s(res_dir));
    args.push("-o".to_owned());
    args.push(s(compiled));
    args
}

/// What `aapt2 link` reads and writes.
pub struct LinkInputs<'a> {
    pub android_jar: &'a Path,
    pub manifest: &'a Path,
    /// The `aapt2 compile` output holding `res/` (linked as a positional input, not an overlay).
    pub compiled_res: &'a Path,
    pub output: &'a Path,
    /// `--debug-mode` (`android:debuggable`), so `adb shell run-as <id>` works.
    pub debug: bool,
}

/// The `aapt2 link` arguments.
pub fn aapt2_link_args(i: &LinkInputs) -> Vec<String> {
    let mut args = strings(&["link", "-o"]);
    args.push(s(i.output));
    args.push("-I".to_owned());
    args.push(s(i.android_jar));
    args.push("--manifest".to_owned());
    args.push(s(i.manifest));
    // A positional input, not `-R`: an overlay may not add resources the base lacks (no `--auto-add-overlay`),
    // and with no base every resource, e.g. the `@color` in values/, is new.
    args.push(s(i.compiled_res));
    args.push("--min-sdk-version".to_owned());
    args.push(MIN_SDK.to_string());
    args.push("--target-sdk-version".to_owned());
    args.push(TARGET_SDK.to_string());
    if i.debug {
        args.push("--debug-mode".to_owned());
    }
    args
}

/// `zipalign -P 16 -f 4 <in> <out>`: 4-byte alignment for everything, 16 KB for stored `.so` files.
pub fn zipalign_args(input: &Path, output: &Path) -> Vec<String> {
    let mut args = strings(&["-P", "16", "-f", "4"]);
    args.push(s(input));
    args.push(s(output));
    args
}

/// `zipalign -c -P 16 -v 4 <apk>`: verifies the alignment.
pub fn zipalign_check_args(apk: &Path) -> Vec<String> {
    let mut args = strings(&["-c", "-P", "16", "-v", "4"]);
    args.push(s(apk));
    args
}

/// The libraries to package: the ABIs asked for, or every ABI that has a built library.
fn collect_libs(target_dir: &Path, requested: &[&str]) -> Result<Vec<NativeLib>, String> {
    let abis = if requested.is_empty() { vec![Abi::Arm64V8a, Abi::X86_64] } else { so::parse_abis_for("apk", requested)? };
    let mut libs = Vec::new();
    for abi in abis {
        let file = so::jni_libs_dir(target_dir, abi).join(LIB_FILE);
        if file.is_file() {
            libs.push(NativeLib { abi: abi.dir().to_owned(), file });
        } else if !requested.is_empty() {
            return Err(format!("android apk: {} not found (run `cargo xtask android so --abi {}` first)", file.display(), abi.dir()));
        }
    }
    if libs.is_empty() {
        return Err(format!(
            "android apk: no {LIB_FILE} under {} (run `cargo xtask android so` first)",
            target_dir.join("android").join("jniLibs").display()
        ));
    }
    Ok(libs)
}

/// The `keytool` to run: `$JAVA_HOME/bin/keytool` when that exists, else the one on `PATH`.
fn keytool() -> PathBuf {
    let exe = if cfg!(windows) { "keytool.exe" } else { "keytool" };
    std::env::var_os("JAVA_HOME").map(|h| PathBuf::from(h).join("bin").join(exe)).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from("keytool"))
}

fn run_tool(program: &Path, args: &[String], what: &str) -> Result<(), String> {
    let mut c = Command::new(program);
    c.args(args);
    crate::run(c, &format!("{what} {}", args.join(" ")))
}

/// How long ago `file` was modified, for the list of packaged libraries (`unknown age` when the
/// file system will not say).
fn built_ago(file: &Path) -> String {
    std::fs::metadata(file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or_else(|| "unknown age".to_owned(), age_text)
}

/// `built 5 min ago`, `built 3 h ago`, `built 2 days ago`.
fn age_text(age: std::time::Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..=59 => "built just now".to_owned(),
        60..=3599 => format!("built {} min ago", secs / 60),
        3600..=86_399 => format!("built {} h ago", secs / 3600),
        _ => format!("built {} days ago", secs / 86_400),
    }
}

/// `cargo xtask android apk …`: writes `<target>/android/lightcraft-debug.apk`.
pub fn run(args: &[&str]) -> Result<(), String> {
    let meta = crate::metadata()?;
    let target_dir = PathBuf::from(meta["target_directory"].as_str().ok_or("cargo metadata: no target_directory")?);
    let android = target_dir.join("android");
    let libs = collect_libs(&target_dir, args)?;

    let sdk_root = sdk::sdk_root(&sdk::process_env)?;
    let jar = android_jar(&sdk_root);
    if !jar.is_file() {
        return Err(format!("{}: not found (sdkmanager \"platforms;android-{TARGET_SDK}\")", jar.display()));
    }
    let tools = build_tools_dir(&sdk_root)?;

    let manifest = manifest::write(Build::Debug, &android)?;
    let work = android.join("apk");
    std::fs::create_dir_all(&work).map_err(|e| format!("create {}: {e}", work.display()))?;
    let (compiled, base, unaligned, aligned) = (work.join("res.zip"), work.join("base.apk"), work.join("unaligned.apk"), work.join("aligned.apk"));
    let signed = android.join("lightcraft-debug.apk");

    let aapt2 = tool(&tools, "aapt2");
    run_tool(&aapt2, &aapt2_compile_args(&crate::root().join("packaging/android/res"), &compiled), "aapt2")?;
    let link = LinkInputs { android_jar: &jar, manifest: &manifest, compiled_res: &compiled, output: &base, debug: true };
    run_tool(&aapt2, &aapt2_link_args(&link), "aapt2")?;
    zip_lib::assemble(&base, &libs, &unaligned)?;
    let zipalign = tool(&tools, "zipalign");
    run_tool(&zipalign, &zipalign_args(&unaligned, &aligned), "zipalign")?;

    let home = signing::home_dir(&sdk::process_env)?;
    let keystore = signing::debug_keystore_path(&home);
    signing::ensure_debug_keystore(&keystore, |a| run_tool(&keytool(), a, "keytool").map_err(|e| format!("{e} (a JDK provides keytool)")))?;
    run_tool(&tool(&tools, "apksigner"), &signing::debug_sign_args(&keystore, &aligned, &signed), "apksigner")?;
    run_tool(&zipalign, &zipalign_check_args(&signed), "zipalign")?;

    for lib in &libs {
        println!("android apk: packaged {} ({})", lib.file.display(), built_ago(&lib.file));
    }
    let size = std::fs::metadata(&signed).map_err(|e| format!("{}: {e}", signed.display()))?.len();
    let abis: Vec<&str> = libs.iter().map(|l| l.abi.as_str()).collect();
    println!("android apk: {} ({size} bytes, {}), install with `adb install -r {}`", signed.display(), abis.join(" + "), signed.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(args: &[String]) -> String {
        args.join(" ")
    }

    #[test]
    fn aapt2_link_args_debug_mode() {
        let (jar, manifest, compiled, out) = (
            Path::new("/sdk/platforms/android-36/android.jar"),
            Path::new("/t/AndroidManifest.xml"),
            Path::new("/t/res.zip"),
            Path::new("/t/base.apk"),
        );
        let debug = joined(&aapt2_link_args(&LinkInputs { android_jar: jar, manifest, compiled_res: compiled, output: out, debug: true }));
        assert!(debug.starts_with("link -o /t/base.apk -I /sdk/platforms/android-36/android.jar"), "{debug}");
        assert!(debug.contains("--manifest /t/AndroidManifest.xml"), "{debug}");
        assert!(debug.contains("--manifest /t/AndroidManifest.xml /t/res.zip --min-sdk-version"), "res/ is linked as an input: {debug}");
        assert!(!debug.contains("-R ") && !debug.contains("--auto-add-overlay"), "no overlay semantics: {debug}");
        assert!(debug.contains("--min-sdk-version 30 --target-sdk-version 36"), "{debug}");
        assert!(debug.ends_with("--debug-mode"), "{debug}");
        let release = joined(&aapt2_link_args(&LinkInputs { android_jar: jar, manifest, compiled_res: compiled, output: out, debug: false }));
        assert!(!release.contains("--debug-mode"), "{release}");
        // the compile step reads the res folder
        let compile = joined(&aapt2_compile_args(Path::new("/repo/packaging/android/res"), compiled));
        assert_eq!(compile, "compile --dir /repo/packaging/android/res -o /t/res.zip");
    }

    #[test]
    fn zipalign_args_use_16k_pages() {
        assert_eq!(joined(&zipalign_args(Path::new("/a.apk"), Path::new("/b.apk"))), "-P 16 -f 4 /a.apk /b.apk");
        assert_eq!(joined(&zipalign_check_args(Path::new("/b.apk"))), "-c -P 16 -v 4 /b.apk");
    }

    #[test]
    fn newest_build_tools_at_least_36() {
        let names: Vec<String> = ["34.0.0", "35.0.1", "36.0.0", "36.1.0", "junk", "37.0.0-rc1"].iter().map(|n| (*n).to_owned()).collect();
        assert_eq!(newest_build_tools(&names).map(String::as_str), Some("36.1.0"), "a stable release beats a newer pre-release");
        assert_eq!(newest_build_tools(&names[..3]).map(String::as_str), Some("36.0.0"));
        let rc: Vec<String> = ["36.0.0-rc1", "36.0.0", "35.0.0"].iter().map(|n| (*n).to_owned()).collect();
        assert_eq!(newest_build_tools(&rc).map(String::as_str), Some("36.0.0"), "the release beats its own candidate");
        assert_eq!(newest_build_tools(&rc[..1]).map(String::as_str), Some("36.0.0-rc1"), "a pre-release is better than nothing");
        assert_eq!(newest_build_tools(&names[..2]), None, "35 is too old for android-36");
        assert!(build_tools_dir(Path::new("/nonexistent-sdk")).unwrap_err().contains("build-tools;36.0.0"));
        assert_eq!(android_jar(Path::new("/sdk")), Path::new("/sdk/platforms/android-36/android.jar"));
    }

    #[test]
    fn library_age_is_readable() {
        use std::time::Duration;
        assert_eq!(age_text(Duration::from_secs(5)), "built just now");
        assert_eq!(age_text(Duration::from_secs(300)), "built 5 min ago");
        assert_eq!(age_text(Duration::from_secs(3 * 3600 + 10)), "built 3 h ago");
        assert_eq!(age_text(Duration::from_secs(2 * 86_400)), "built 2 days ago");
        assert_eq!(built_ago(Path::new("/nonexistent/lib.so")), "unknown age");
    }

    #[test]
    fn libraries_come_from_the_so_step() {
        let dir = std::env::temp_dir().join(format!("xtask-apk-libs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let none = collect_libs(&dir, &[]).unwrap_err();
        assert!(none.contains("android so"), "{none}");
        let arm = so::jni_libs_dir(&dir, Abi::Arm64V8a);
        std::fs::create_dir_all(&arm).unwrap();
        std::fs::write(arm.join(LIB_FILE), b"so").unwrap();
        // no --abi: every built ABI; here only arm64-v8a exists
        let libs = collect_libs(&dir, &[]).unwrap();
        assert_eq!(libs.iter().map(|l| l.abi.as_str()).collect::<Vec<_>>(), ["arm64-v8a"]);
        assert_eq!(libs[0].entry_name().unwrap(), "lib/arm64-v8a/liblightcraft_android.so");
        // asking for an ABI that was not built is an error that names the fix
        let err = collect_libs(&dir, &["--abi", "x86_64"]).unwrap_err();
        assert!(err.contains("--abi x86_64"), "{err}");
        let bogus = collect_libs(&dir, &["--bogus"]).unwrap_err();
        assert!(bogus.starts_with("android apk:"), "{bogus}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
