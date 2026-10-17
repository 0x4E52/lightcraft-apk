//! Checks on the linked `liblightcraft_android.so`, from the NDK's `llvm-readelf -lW -d --dyn-syms`
//! text: 16 KB page alignment, the two symbols Android looks up, and only system libraries needed.
//! The parsers are pure so the checks run on canned output.

use std::path::Path;
use std::process::Command;

/// Page size Google Play requires `LOAD` segments to support from November 2025.
const MIN_LOAD_ALIGN: u64 = 0x4000;

/// Symbols the platform resolves: the Java `NativeActivity` calls the first, android-activity
/// provides the second and calls `android_main`.
const ENTRY_SYMBOLS: [&str; 2] = ["android_main", "ANativeActivity_onCreate"];

/// Libraries every Android system image has (and the NDK links against). Anything else, notably
/// `libc++_shared.so`, would be missing from the APK and fail at `dlopen` on a device.
const SYSTEM_LIBS: [&str; 9] =
    ["libc.so", "libm.so", "libdl.so", "liblog.so", "libandroid.so", "libnativewindow.so", "libvulkan.so", "libEGL.so", "libGLESv2.so"];

/// The `Align` of every `LOAD` program header.
fn load_alignments(readelf: &str) -> Vec<u64> {
    readelf
        .lines()
        .map(str::trim)
        .filter(|l| l.split_whitespace().next() == Some("LOAD"))
        .filter_map(|l| l.split_whitespace().last())
        .map(|a| a.strip_prefix("0x").map_or_else(|| a.parse().ok(), |h| u64::from_str_radix(h, 16).ok()).unwrap_or(0))
        .collect()
}

/// The library names of the `NEEDED` dynamic entries.
fn needed_libs(readelf: &str) -> Vec<String> {
    readelf
        .lines()
        .filter(|l| l.contains("(NEEDED)"))
        .filter_map(|l| {
            let (_, rest) = l.split_once('[')?;
            Some(rest.split_once(']')?.0.to_owned())
        })
        .collect()
}

/// Names of the defined, global (or weak) dynamic symbols.
fn exported_symbols(readelf: &str) -> Vec<String> {
    readelf
        .lines()
        .filter_map(|l| {
            // `    12: 00000000001f2340   120 FUNC    GLOBAL DEFAULT    15 android_main`
            let f: Vec<&str> = l.split_whitespace().collect();
            let (num, rest) = f.split_first()?;
            num.strip_suffix(':')?.parse::<u32>().ok()?;
            let [_value, _size, _kind, bind, vis, ndx, name] = <[&str; 7]>::try_from(rest.get(..7)?).ok()?;
            let defined = ndx != "UND" && matches!(bind, "GLOBAL" | "WEAK") && vis == "DEFAULT";
            defined.then(|| name.split('@').next().unwrap_or(name).to_owned())
        })
        .collect()
}

/// Every `LOAD` segment is aligned to a power of two of at least 16 KB.
pub fn load_alignment_ok(readelf: &str) -> Result<(), String> {
    let aligns = load_alignments(readelf);
    if aligns.is_empty() {
        return Err("the readelf output lists no LOAD segment".into());
    }
    match aligns.iter().find(|a| !a.is_power_of_two() || **a < MIN_LOAD_ALIGN) {
        Some(a) => Err(format!(
            "a LOAD segment is aligned to {a:#x}; Android 15+ devices with 16 KB pages need at least {MIN_LOAD_ALIGN:#x} (-Wl,-z,max-page-size=16384)"
        )),
        None => Ok(()),
    }
}

/// `android_main` and `ANativeActivity_onCreate` are exported.
pub fn requires_entry_symbols(readelf: &str) -> Result<(), String> {
    let exported = exported_symbols(readelf);
    let missing: Vec<&str> = ENTRY_SYMBOLS.iter().copied().filter(|s| !exported.iter().any(|e| e == s)).collect();
    if missing.is_empty() { Ok(()) } else { Err(format!("the library does not export {}", missing.join(", "))) }
}

/// Every `NEEDED` library is one the system provides.
pub fn needed_libs_are_system(readelf: &str) -> Result<(), String> {
    let other: Vec<String> = needed_libs(readelf).into_iter().filter(|l| !SYSTEM_LIBS.contains(&l.as_str())).collect();
    if other.is_empty() {
        Ok(())
    } else {
        Err(format!("the library needs {}, which Android does not provide (a dependency links C++ or a non-NDK library)", other.join(", ")))
    }
}

/// All three checks on one readelf output.
pub fn check_text(readelf: &str) -> Result<(), String> {
    load_alignment_ok(readelf)?;
    requires_entry_symbols(readelf)?;
    needed_libs_are_system(readelf)
}

/// Run the NDK's `llvm-readelf` on `so` and check the output.
pub fn check(llvm_bin: &Path, so: &Path) -> Result<(), String> {
    let tool = llvm_bin.join("llvm-readelf");
    let out = Command::new(&tool).args(["-lW", "-d", "--dyn-syms"]).arg(so).output().map_err(|e| format!("{}: {e}", tool.display()))?;
    if !out.status.success() {
        return Err(format!("llvm-readelf {} failed: {}", so.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    check_text(&String::from_utf8_lossy(&out.stdout)).map_err(|e| format!("{}: {e}", so.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `llvm-readelf -lW -d --dyn-syms` of a healthy library (trimmed), with the LOAD alignment and
    /// the NEEDED list as parameters.
    fn sample(align: &str, needed: &[&str], symbols: &[&str]) -> String {
        let mut s = String::from(
            "\nElf file type is DYN (Shared object file)\nProgram Headers:\n  Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align\n  PHDR           0x000040 0x0000000000000040 0x0000000000000040 0x000230 0x000230 R   0x8\n",
        );
        for flags in ["R  ", "R E", "RW ", "RW "] {
            s.push_str(&format!("  LOAD           0x000000 0x0000000000000000 0x0000000000000000 0x0a1234 0x0a1234 {flags} {align}\n"));
        }
        s.push_str("  DYNAMIC        0x9b0c10 0x00000000009b1c10 0x00000000009b1c10 0x000210 0x000210 RW  0x8\n");
        s.push_str("\nDynamic section at offset 0x9b0c10 contains 3 entries:\n  Tag                Type                         Name/Value\n");
        for n in needed {
            s.push_str(&format!("  0x0000000000000001 (NEEDED)                     Shared library: [{n}]\n"));
        }
        s.push_str("  0x000000000000000e (SONAME)                     Library soname: [liblightcraft_android.so]\n");
        s.push_str("\nSymbol table '.dynsym' contains 4 entries:\n   Num:    Value          Size Type    Bind   Vis       Ndx Name\n     0: 0000000000000000     0 NOTYPE  LOCAL  DEFAULT   UND \n     1: 0000000000000000     0 FUNC    GLOBAL DEFAULT   UND malloc\n");
        for (i, name) in symbols.iter().enumerate() {
            s.push_str(&format!("    {}: 00000000001f2340   120 FUNC    GLOBAL DEFAULT    15 {name}\n", i + 2));
        }
        s
    }

    const GOOD_LIBS: [&str; 4] = ["libc.so", "libm.so", "libdl.so", "liblog.so"];
    const ENTRY: [&str; 2] = ["android_main", "ANativeActivity_onCreate"];

    #[test]
    fn rejects_4k_load_alignment() {
        let err = load_alignment_ok(&sample("0x1000", &GOOD_LIBS, &ENTRY)).unwrap_err();
        assert!(err.contains("0x1000") && err.contains("16 KB"), "{err}");
        assert!(check_text(&sample("0x1000", &GOOD_LIBS, &ENTRY)).is_err());
        // one bad segment among good ones is still a failure, and so is an alignment that is no power of two
        let mixed = sample("0x10000", &GOOD_LIBS, &ENTRY).replacen("0x10000", "0x1000", 1);
        assert!(load_alignment_ok(&mixed).is_err());
        assert!(load_alignment_ok(&sample("0x6000", &GOOD_LIBS, &ENTRY)).is_err());
        assert!(load_alignment_ok("no program headers here").is_err(), "no LOAD segment is an error");
    }

    #[test]
    fn accepts_64k_load_alignment() {
        assert_eq!(load_alignment_ok(&sample("0x10000", &GOOD_LIBS, &ENTRY)), Ok(()));
        assert_eq!(load_alignment_ok(&sample("0x4000", &GOOD_LIBS, &ENTRY)), Ok(()));
        assert_eq!(check_text(&sample("0x10000", &GOOD_LIBS, &ENTRY)), Ok(()));
    }

    #[test]
    fn requires_entry_symbols() {
        assert_eq!(super::requires_entry_symbols(&sample("0x4000", &GOOD_LIBS, &ENTRY)), Ok(()));
        let err = super::requires_entry_symbols(&sample("0x4000", &GOOD_LIBS, &["android_main"])).unwrap_err();
        assert!(err.contains("ANativeActivity_onCreate") && !err.contains("android_main"), "{err}");
        // an undefined or local symbol of that name does not count
        let undefined = sample("0x4000", &GOOD_LIBS, &[])
            + "    9: 0000000000000000     0 FUNC    GLOBAL DEFAULT   UND android_main\n   10: 00000000001f2340   8 FUNC    LOCAL  DEFAULT    15 ANativeActivity_onCreate\n";
        assert!(super::requires_entry_symbols(&undefined).is_err());
        // a symbol version suffix is dropped
        let versioned = sample("0x4000", &GOOD_LIBS, &["android_main@@LIB", "ANativeActivity_onCreate"]);
        assert_eq!(super::requires_entry_symbols(&versioned), Ok(()));
    }

    #[test]
    fn rejects_libcxx_shared() {
        let mut libs = GOOD_LIBS.to_vec();
        libs.push("libc++_shared.so");
        let err = needed_libs_are_system(&sample("0x4000", &libs, &ENTRY)).unwrap_err();
        assert!(err.contains("libc++_shared.so") && !err.contains("libm.so"), "{err}");
        let all = ["libc.so", "libm.so", "libdl.so", "liblog.so", "libandroid.so", "libnativewindow.so", "libvulkan.so", "libEGL.so", "libGLESv2.so"];
        assert_eq!(needed_libs_are_system(&sample("0x4000", &all, &ENTRY)), Ok(()));
        assert!(needed_libs_are_system(&sample("0x4000", &["libz.so"], &ENTRY)).is_err());
    }
}
