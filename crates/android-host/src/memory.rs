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
    fn mem_total_reads_bytes() {
        assert_eq!(mem_total(&meminfo(3_785_940)), Some(3_785_940 * 1024));
        // the unit may be absent on odd kernels
        assert_eq!(mem_total("MemTotal: 2048\n"), Some(2048 * 1024));
        assert_eq!(mem_total("garbage"), None);
    }
}
