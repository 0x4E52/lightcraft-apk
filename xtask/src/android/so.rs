//! `cargo xtask android so [--abi arm64-v8a|x86_64]…`: link `liblightcraft_android.so` with the NDK's
//! clang (no `cargo ndk`), copy it to `<target>/android/jniLibs/<abi>/` and check it (elf.rs).
//! Needs the NDK (sdk.rs); the pure parts (linker path, flags, arguments) are unit-tested.

use std::path::{Path, PathBuf};

use super::elf;
use super::sdk::{self, MIN_SDK, Ndk, NdkVersion};

/// The package whose cdylib is the app's library.
const PACKAGE: &str = "lightcraft-android";

/// File name of the library (the `[lib] name` of `lightcraft-android`).
pub(super) const LIB_FILE: &str = "liblightcraft_android.so";

/// First NDK whose linker defaults to 16 KB pages; older ones need the flag below.
const NDK_DEFAULT_16K: u32 = 28;

/// Flag that makes the linker lay segments out for 16 KB pages.
const PAGE_SIZE_FLAG: &str = "-C link-arg=-Wl,-z,max-page-size=16384";

/// An Android ABI we build for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abi {
    /// Release phones.
    Arm64V8a,
    /// The CI emulator only (decision 5).
    X86_64,
}

impl Abi {
    /// The name Android uses for the `lib/<abi>/` folder.
    pub fn dir(self) -> &'static str {
        match self {
            Abi::Arm64V8a => "arm64-v8a",
            Abi::X86_64 => "x86_64",
        }
    }

    /// The Rust target triple (and the NDK clang's prefix).
    pub fn triple(self) -> &'static str {
        match self {
            Abi::Arm64V8a => "aarch64-linux-android",
            Abi::X86_64 => "x86_64-linux-android",
        }
    }

    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "arm64-v8a" => Ok(Abi::Arm64V8a),
            "x86_64" => Ok(Abi::X86_64),
            other => Err(format!("android so: unknown ABI `{other}` (arm64-v8a or x86_64)")),
        }
    }
}

/// `--abi <name>` (repeatable) → the ABIs to build, in order and without repeats; arm64-v8a alone
/// when none is given.
pub fn parse_abis(args: &[&str]) -> Result<Vec<Abi>, String> {
    parse_abis_for("so", args)
}

/// [`parse_abis`] with errors that name the xtask step (`so`, `apk`) the arguments were given to.
pub fn parse_abis_for(step: &str, args: &[&str]) -> Result<Vec<Abi>, String> {
    let mut abis: Vec<Abi> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg != "--abi" {
            return Err(format!("android {step}: unknown argument `{arg}` (usage: cargo xtask android {step} [--abi arm64-v8a|x86_64]…)"));
        }
        let value = it.next().ok_or_else(|| format!("android {step}: --abi needs a value (arm64-v8a or x86_64)"))?;
        let abi = Abi::parse(value)?;
        if !abis.contains(&abi) {
            abis.push(abi);
        }
    }
    if abis.is_empty() {
        abis.push(Abi::Arm64V8a);
    }
    Ok(abis)
}

/// The NDK's clang wrapper for `abi` and API level `min_sdk`, which also selects the right sysroot.
pub fn linker_path(llvm_bin: &Path, abi: Abi, min_sdk: u32) -> PathBuf {
    llvm_bin.join(format!("{}{min_sdk}-clang", abi.triple()))
}

/// The environment variable cargo reads the target's linker from.
fn linker_env(abi: Abi) -> String {
    format!("CARGO_TARGET_{}_LINKER", abi.triple().to_uppercase().replace('-', "_"))
}

/// `RUSTFLAGS` for the build: `existing` plus the 16 KB page flag when the NDK predates r28, or
/// `None` when the environment needs no change.
pub fn rustflags(existing: Option<&str>, ndk: NdkVersion) -> Option<String> {
    if ndk.major >= NDK_DEFAULT_16K {
        return None;
    }
    Some(match existing.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => format!("{e} {PAGE_SIZE_FLAG}"),
        None => PAGE_SIZE_FLAG.to_owned(),
    })
}

/// Where the library for `abi` goes for the packaging step.
pub fn jni_libs_dir(target_dir: &Path, abi: Abi) -> PathBuf {
    target_dir.join("android").join("jniLibs").join(abi.dir())
}

fn build(ndk: &Ndk, abi: Abi, target_dir: &Path) -> Result<PathBuf, String> {
    let linker = linker_path(&sdk::llvm_bin(&ndk.root), abi, MIN_SDK);
    if !linker.is_file() {
        return Err(format!("{}: not found (is the NDK at {} complete?)", linker.display(), ndk.root.display()));
    }
    let mut c = crate::cargo();
    c.args(["build", "-p", PACKAGE, "--target", abi.triple()]).env(linker_env(abi), &linker);
    if let Some(flags) = rustflags(std::env::var("RUSTFLAGS").ok().as_deref(), ndk.version) {
        c.env("RUSTFLAGS", flags);
    }
    crate::run(c, &format!("cargo build -p {PACKAGE} --target {}", abi.triple()))?;

    let built = target_dir.join(abi.triple()).join("debug").join(LIB_FILE);
    let dir = jni_libs_dir(target_dir, abi);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(LIB_FILE);
    std::fs::copy(&built, &dest).map_err(|e| format!("{} → {}: {e}", built.display(), dest.display()))?;
    Ok(dest)
}

/// `cargo xtask android so …`.
pub fn run(args: &[&str]) -> Result<(), String> {
    let abis = parse_abis(args)?;
    let ndk = sdk::ndk(&sdk::process_env)?;
    let meta = crate::metadata()?;
    let target_dir = PathBuf::from(meta["target_directory"].as_str().ok_or("cargo metadata: no target_directory")?);
    for abi in abis {
        let so = build(&ndk, abi, &target_dir)?;
        elf::check(&sdk::llvm_bin(&ndk.root), &so)?;
        let size = std::fs::metadata(&so).map_err(|e| format!("{}: {e}", so.display()))?.len();
        println!("android so {}: {} ({size} bytes), 16 KB aligned, entry symbols exported, system libraries only", abi.dir(), so.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linker_path_for_abi() {
        let bin = sdk::llvm_bin(Path::new("/ndk"));
        let arm = linker_path(&bin, Abi::Arm64V8a, 30);
        assert_eq!(arm.file_name().and_then(|n| n.to_str()), Some("aarch64-linux-android30-clang"));
        assert_eq!(arm.parent(), Some(bin.as_path()));
        assert!(arm.starts_with("/ndk/toolchains/llvm/prebuilt"), "{arm:?}");
        assert_eq!(linker_path(&bin, Abi::X86_64, 30).file_name().and_then(|n| n.to_str()), Some("x86_64-linux-android30-clang"));
        assert_eq!(linker_env(Abi::Arm64V8a), "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER");
        assert_eq!(linker_env(Abi::X86_64), "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER");
        assert_eq!(jni_libs_dir(Path::new("/t"), Abi::X86_64), Path::new("/t/android/jniLibs/x86_64"));
    }

    #[test]
    fn abis_default_to_arm64_and_do_not_repeat() {
        assert_eq!(parse_abis(&[]), Ok(vec![Abi::Arm64V8a]));
        assert_eq!(parse_abis(&["--abi", "arm64-v8a", "--abi", "x86_64", "--abi", "arm64-v8a"]), Ok(vec![Abi::Arm64V8a, Abi::X86_64]));
        assert!(parse_abis(&["--abi", "armeabi-v7a"]).is_err());
        assert!(parse_abis(&["--abi"]).is_err());
        assert!(parse_abis(&["x86_64"]).is_err());
        let apk = parse_abis_for("apk", &["--bogus"]).unwrap_err();
        assert!(apk.starts_with("android apk:") && apk.contains("cargo xtask android apk"), "{apk}");
    }

    #[test]
    fn page_size_flag_only_before_ndk_28() {
        let v = |major| NdkVersion { major, minor: 0, patch: 0 };
        assert_eq!(rustflags(None, v(28)), None);
        assert_eq!(rustflags(Some("-Dwarnings"), v(29)), None);
        assert_eq!(rustflags(None, v(27)).as_deref(), Some("-C link-arg=-Wl,-z,max-page-size=16384"));
        assert_eq!(rustflags(Some("-Dwarnings"), v(27)).as_deref(), Some("-Dwarnings -C link-arg=-Wl,-z,max-page-size=16384"));
        assert_eq!(rustflags(Some("  "), v(26)).as_deref(), Some(PAGE_SIZE_FLAG));
    }
}
