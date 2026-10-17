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
    fn mem_total_reads_bytes() {
        assert_eq!(mem_total(&meminfo(3_785_940)), Some(3_785_940 * 1024));
        // the unit may be absent on odd kernels
        assert_eq!(mem_total("MemTotal: 2048\n"), Some(2048 * 1024));
        assert_eq!(mem_total("garbage"), None);
    }
}
