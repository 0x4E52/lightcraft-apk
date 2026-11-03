//! A memory budget a phone can hold.
//!
//! Android's low-memory killer ends a process that holds too much native memory, without a signal
//! the panic guard could catch, so the host gives the engine a budget sized for the device:
//! an eighth of its physical memory, between 256 MiB and 768 MiB (the desktop takes a quarter, up
//! to 1.5 GiB; the engine's caches take half the budget and an export batch up to twice it). The
//! physical memory comes from `/proc/meminfo` (`MemTotal`); `ActivityManager.getMemoryClass()`
//! is not used, as it bounds the Java heap, not native allocations. The same figure goes to
//! `lightcraft_engine::memory::set_total_ram`, so export and merge check what fits (the engine
//! reads `/proc/meminfo` only on desktop Linux).

/// The smallest budget, also used when `/proc/meminfo` can't be read.
pub const MIN_BUDGET: usize = 256 << 20;
/// The largest budget, whatever the device has.
pub const MAX_BUDGET: usize = 768 << 20;

/// The least `MemTotal` that gets Object and Describe masks (SAM 3's model holds several GB, and
/// the low-memory killer ends the process without a word). The UI says "8 GB": a phone sold with
/// 8 GB reads 7.2 to 7.9 GiB here (the kernel and firmware keep the rest), so the line is 7 GiB,
/// which still keeps out the 6 GB phones (about 5.5 GiB).
pub const SAM_MIN_RAM: u64 = 7 << 30;

/// The least `MemTotal` that gets AI Denoise (a 24 MP photo holds its mosaic, the denoised picture and the tiles,
/// over half a GB besides the app, and the low-memory killer ends the process without a word). The UI says "6 GB": a
/// phone sold with 6 GB reads about 5.5 GiB here, so the line is 5 GiB, which keeps out the 4 GB phones (about
/// 3.6 GiB). The tiles themselves follow the budget (`lightcraft_denoise::budget`), so 6 and 8 GB phones differ in
/// speed, not in whether it runs.
pub const DENOISE_MIN_RAM: u64 = 5 << 30;

/// Whether a device with this `MemTotal` (bytes; `None` = unreadable) gets SAM-based masks. A
/// device that can't tell its memory does not.
pub fn sam_allowed(mem_total: Option<u64>) -> bool {
    mem_total.is_some_and(|total| total >= SAM_MIN_RAM)
}

/// `MemTotal` from the text of `/proc/meminfo`, in bytes; `None` when it is missing or garbled.
pub fn mem_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
    let mut words = line.split_whitespace();
    let kb: u64 = words.next()?.parse().ok()?;
    // the kernel always writes kB; anything else is not a meminfo we understand
    if words.next().is_some_and(|unit| unit != "kB") {
        return None;
    }
    kb.checked_mul(1024).filter(|b| *b > 0)
}

/// The memory budget for a device whose `/proc/meminfo` reads `meminfo`: `MemTotal` / 8, clamped
/// to [`MIN_BUDGET`]..=[`MAX_BUDGET`]; [`MIN_BUDGET`] when `MemTotal` can't be read.
pub fn budget_for(meminfo: &str) -> usize {
    let Some(total) = mem_total(meminfo) else { return MIN_BUDGET };
    let eighth = usize::try_from(total / 8).unwrap_or(MAX_BUDGET);
    eighth.clamp(MIN_BUDGET, MAX_BUDGET)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1 << 20;

    fn meminfo(kb: u64) -> String {
        format!("MemTotal:       {kb} kB\nMemFree:          123456 kB\nMemAvailable:    2345678 kB\n")
    }

    #[test]
    fn budget_for_phone_meminfo() {
        // 4 GB phone: an eighth
        assert_eq!(budget_for(&meminfo(4 << 20)), 512 * MIB);
        // 12 GB phone: capped
        assert_eq!(budget_for(&meminfo(12 << 20)), 768 * MIB);
        // 1.5 GB: floored
        assert_eq!(budget_for(&meminfo(1536 << 10)), 256 * MIB);
        // garbage, a missing line, a wrong unit, zero: the floor
        for bad in ["", "garbage", "MemFree: 100 kB\n", "MemTotal: lots kB\n", "MemTotal: 4194304 MB\n", "MemTotal: 0 kB\n", "MemTotal:\n"] {
            assert_eq!(budget_for(bad), 256 * MIB, "{bad:?}");
        }
        // hostile numbers neither overflow nor panic
        assert_eq!(mem_total(&format!("MemTotal: {} kB\n", u64::MAX)), None);
        assert_eq!(budget_for(&format!("MemTotal: {} kB\n", u64::MAX / 1024)), 768 * MIB);
    }

    #[test]
    fn sam_allowed_for_phone_meminfo() {
        // 12 GB, 8 GB (reads about 7.5 GiB) and 16 GB phones: allowed
        for kb in [12u64 << 20, 7_700_000, 16 << 20] {
            assert!(sam_allowed(mem_total(&meminfo(kb))), "{kb} kB");
        }
        // 6 GB (about 5.5 GiB), 4 GB and 2 GB phones: not
        for kb in [5_800_000u64, 4 << 20, 2 << 20] {
            assert!(!sam_allowed(mem_total(&meminfo(kb))), "{kb} kB");
        }
        // unreadable: not, and the edge is exact
        assert!(!sam_allowed(mem_total("garbage")));
        assert!(sam_allowed(Some(SAM_MIN_RAM)) && !sam_allowed(Some(SAM_MIN_RAM - 1)));
        // the UI's check agrees with the host's for the same memory
        let reason = |kb: u64| lightcraft_ui_egui::panels::masking::sam_ram_reason(Some(SAM_MIN_RAM), mem_total(&meminfo(kb)));
        assert!(reason(12 << 20).is_none() && reason(4 << 20).is_some());
        assert!(lightcraft_ui_egui::panels::masking::sam_ram_reason(Some(SAM_MIN_RAM), None).is_some());
    }

    #[test]
    fn denoise_allowed_for_phone_meminfo() {
        let reason = |kb: u64| lightcraft_engine::denoise::ram_reason(Some(DENOISE_MIN_RAM), mem_total(&meminfo(kb)));
        // 6 GB (about 5.5 GiB), 8 GB and 12 GB phones: allowed
        for kb in [5_800_000u64, 7_700_000, 12 << 20] {
            assert!(reason(kb).is_none(), "{kb} kB");
        }
        // 4 GB (about 3.6 GiB) and 2 GB phones, and an unreadable meminfo: not, saying the class it needs
        for kb in [3_800_000u64, 2 << 20] {
            assert_eq!(reason(kb).as_deref(), Some("AI Denoise needs a device with 6 GB of memory"), "{kb} kB");
        }
        assert!(lightcraft_engine::denoise::ram_reason(Some(DENOISE_MIN_RAM), mem_total("garbage")).is_some());
    }

    #[test]
    fn denoise_reason_for_the_floor_is_translated() {
        // `ram_reason` words the class from the floor; the locales translate only the sentence this floor gives, so a
        // new floor must come with its translations (the UI shows it as a sentence, with a full stop)
        let why = lightcraft_engine::denoise::ram_reason(Some(DENOISE_MIN_RAM), Some(0)).unwrap();
        let shown = format!("{why}.");
        for locale in lightcraft_ui_egui::i18n::Locale::ALL.iter().filter(|l| **l != lightcraft_ui_egui::i18n::Locale::En) {
            assert_ne!(locale.tr(&shown), shown, "{} has no entry for {shown:?}", locale.code());
        }
    }

    #[test]
    fn mem_total_reads_bytes() {
        assert_eq!(mem_total(&meminfo(3_785_940)), Some(3_785_940 * 1024));
        // the unit may be absent on odd kernels
        assert_eq!(mem_total("MemTotal: 2048\n"), Some(2048 * 1024));
        assert_eq!(mem_total("garbage"), None);
    }
}
