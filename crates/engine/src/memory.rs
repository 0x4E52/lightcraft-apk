//! Memory accounting: what each cache holds (`library.memory`, and `ui.inspect` → `memory` with the
//! frontend's own caches added).
//!
//! The numbers are the caches' own bookkeeping (pixels held), not the process's resident size: the
//! allocator keeps freed pages for reuse and the GPU driver maps device buffers, so `ps`/`time -l`
//! report more. A binary built with a heap profiler installs [`set_heap_stats`] (e.g.
//! `lightcraft-cli` with `--features dhat-heap`) to add live/peak heap bytes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use serde::Serialize;

/// The memory budget: one number from which every cache's share is derived.
///
/// - half for the engine's caches (decoded sources and rendered previews), evicted least recently
///   used *across* them ([`crate::media::MediaCache`]);
/// - a quarter for the working memory of decodes in flight ([`work_gate`]);
/// - an eighth for the GPU renderer's pool of recycled buffers (trimmed when the app is idle).
///
/// The rest is headroom for what isn't cached (renders in progress, textures, the UI). An export
/// batch may, in addition, hold up to twice the budget in photos rendering side by side
/// ([`export_gate`]): transient memory, given back when the batch ends. Default
/// ("automatic"): `LIGHTCRAFT_MEMORY_MB` when set, else what the host gave
/// [`set_default_budget`] (Android: a phone-sized budget), else a quarter of the machine's RAM,
/// at most 1.5 GiB.
pub fn default_budget() -> usize {
    let env = std::env::var("LIGHTCRAFT_MEMORY_MB").ok();
    default_budget_from(env.as_deref(), DEFAULT_BUDGET.get().copied(), total_ram)
}

/// The automatic budget a host gave with [`set_default_budget`]. Process-wide, set at most once.
static DEFAULT_BUDGET: OnceLock<usize> = OnceLock::new();

/// Tell the engine the automatic budget (bytes, at least 64 MiB) for hosts whose devices can't
/// spare the desktop's quarter of RAM (Android). Settings ▸ Performance ▸ Memory set to Automatic,
/// the bug report and [`budget`] before any [`set_budget`] then use it. Call it once at start-up.
/// The same value again is `Ok`; a different one is refused with `Err(the value in use)`.
pub fn set_default_budget(bytes: usize) -> Result<(), usize> {
    set_once_in(&DEFAULT_BUDGET, bytes.max(64 << 20))
}

/// [`default_budget`] over a given environment value, injected budget and RAM probe.
fn default_budget_from(env_mb: Option<&str>, injected: Option<usize>, ram: impl FnOnce() -> Option<usize>) -> usize {
    if let Some(mb) = env_mb.and_then(|v| v.trim().parse::<usize>().ok()).filter(|m| *m >= 64) {
        return mb.saturating_mul(1 << 20);
    }
    if let Some(b) = injected {
        return b;
    }
    let cap = 3usize << 29; // 1.5 GiB
    match ram() {
        Some(ram) => (ram / 4).clamp(256 << 20, cap),
        None => cap,
    }
}

/// Physical memory (bytes): what the host gave [`set_total_ram`], else what the platform tells us
/// without native calls (`None` where it doesn't, e.g. Android, whose host injects it). Export's and
/// merge's memory fit checks use it.
pub fn total_ram() -> Option<usize> {
    total_ram_in(&TOTAL_RAM, probe_total_ram)
}

/// The physical memory a host gave with [`set_total_ram`]. Process-wide, set at most once.
static TOTAL_RAM: OnceLock<usize> = OnceLock::new();

/// Tell the engine how much physical memory the device has (bytes), for hosts where
/// [`total_ram`] can't find out itself (Android: `/proc/meminfo` is read by the host, the engine
/// has no Android code). Call it once at start-up. The same value again is `Ok`; a different one is
/// refused with `Err(the value in use)`, and the first one stays.
pub fn set_total_ram(bytes: usize) -> Result<(), usize> {
    set_once_in(&TOTAL_RAM, bytes)
}

/// [`set_total_ram`] / [`set_default_budget`] on a given cell (tests use a local one, never the
/// global).
fn set_once_in(cell: &OnceLock<usize>, bytes: usize) -> Result<(), usize> {
    let current = *cell.get_or_init(|| bytes);
    if current == bytes { Ok(()) } else { Err(current) }
}

/// [`total_ram`] over a given cell and platform probe: the injected value first.
fn total_ram_in(cell: &OnceLock<usize>, probe: impl FnOnce() -> Option<usize>) -> Option<usize> {
    cell.get().copied().or_else(probe)
}

/// Physical memory (bytes), when the platform tells us without native calls.
fn probe_total_ram() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: usize = s.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse().ok()?;
        kb.checked_mul(1024)
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(target_os = "freebsd")]
    {
        let out = std::process::Command::new("/sbin/sysctl").args(["-n", "hw.physmem"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
    {
        None
    }
}

static BUDGET: AtomicUsize = AtomicUsize::new(0);

/// The current budget (bytes).
pub fn budget() -> usize {
    match BUDGET.load(Ordering::Relaxed) {
        0 => {
            let b = default_budget();
            BUDGET.store(b, Ordering::Relaxed);
            apply(b);
            b
        }
        b => b,
    }
}

/// Change the budget (bytes, at least 64 MiB) for the process: the decode gate and the GPU pool
/// follow at once; sessions apply their cache share with [`crate::Session::set_memory_budget`].
pub fn set_budget(bytes: usize) -> usize {
    let b = bytes.max(64 << 20);
    BUDGET.store(b, Ordering::Relaxed);
    apply(b);
    b
}

fn apply(b: usize) {
    if let Some(g) = GATE.get() {
        g.set_limit(b / 4);
    }
    if let Some(g) = EXPORT_GATE.get() {
        g.set_limit(export_share(b));
    }
    lightcraft_gpu::set_pool_limit((b / 8) as u64);
}

/// Share of the budget for the engine's caches.
pub fn cache_share(budget: usize) -> usize {
    budget / 2
}

/// A weighted semaphore over bytes of working memory: [`WorkGate::acquire`] waits while other
/// holders already use the limit (one holder may always proceed, whatever its size).
pub struct WorkGate {
    state: Mutex<(usize, usize)>,
    cv: Condvar,
}

/// Bytes held from a [`WorkGate`] until dropped.
pub struct Permit<'a> {
    gate: &'a WorkGate,
    bytes: usize,
}

impl WorkGate {
    pub fn new(limit: usize) -> WorkGate {
        WorkGate { state: Mutex::new((0, limit)), cv: Condvar::new() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (usize, usize)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_limit(&self, limit: usize) {
        self.lock().1 = limit;
        self.cv.notify_all();
    }

    /// Bytes held now and the limit.
    pub fn usage(&self) -> (usize, usize) {
        *self.lock()
    }

    /// Hold `bytes`, waiting until they fit (or nothing else is held).
    pub fn acquire(&self, bytes: usize) -> Permit<'_> {
        let mut g = self.lock();
        while g.0 > 0 && g.0 + bytes > g.1 {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        g.0 += bytes;
        Permit { gate: self, bytes }
    }

    /// Hold `bytes` without waiting (interactive work): counted, so background work waits for it.
    pub fn acquire_urgent(&self, bytes: usize) -> Permit<'_> {
        self.lock().0 += bytes;
        Permit { gate: self, bytes }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut g = self.gate.lock();
        g.0 = g.0.saturating_sub(self.bytes);
        drop(g);
        self.gate.cv.notify_all();
    }
}

thread_local! {
    static BACKGROUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` as background work (grid thumbnails, neighbour prefetch): its decodes wait at the
/// [`work_gate`] while interactive work holds the memory.
pub fn in_background<R>(f: impl FnOnce() -> R) -> R {
    let was = BACKGROUND.with(|b| b.replace(true));
    let r = f();
    BACKGROUND.with(|b| b.set(was));
    r
}

/// Is this thread running background work ([`in_background`])?
pub fn is_background() -> bool {
    BACKGROUND.with(|b| b.get())
}

/// The process-wide gate for decodes and import probes (limit: a quarter of the budget).
pub fn work_gate() -> &'static WorkGate {
    GATE.get_or_init(|| WorkGate::new(budget() / 4))
}

static GATE: OnceLock<WorkGate> = OnceLock::new();

/// The process-wide gate for the photos of an export batch rendering side by side
/// ([`crate::export::run_batch`]; limit: twice the budget). Separate from [`work_gate`], which the
/// decodes inside each export take: one gate held across the other's acquire could deadlock.
pub fn export_gate() -> &'static WorkGate {
    EXPORT_GATE.get_or_init(|| WorkGate::new(export_share(budget())))
}

fn export_share(budget: usize) -> usize {
    budget.saturating_mul(2)
}

static EXPORT_GATE: OnceLock<WorkGate> = OnceLock::new();

/// Entries and bytes held by one cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub count: usize,
    pub bytes: usize,
}

impl Usage {
    pub fn new(count: usize, bytes: usize) -> Usage {
        Usage { count, bytes }
    }
}

/// Heap bytes as counted by an instrumented allocator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeapUsage {
    /// Live heap bytes now.
    pub current: u64,
    /// Highest live heap bytes so far.
    pub peak: u64,
}

/// Device buffers of the GPU renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuUsage {
    /// Every device buffer that exists.
    pub allocated: u64,
    /// Of which recycled buffers waiting in the free pool.
    pub pooled: u64,
    /// Of which buffers released since their thread's last submit.
    pub retired: u64,
}

/// What the engine's caches hold.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReport {
    /// Decoded thumbnail-level sources (≤ 512 px, linear float).
    pub thumb_sources: Usage,
    /// Decoded preview-level sources (≤ 2560 px, linear float) of the current and prefetched photos.
    pub preview_sources: Usage,
    /// The last full-resolution original (exports, 1:1).
    pub full_source: Usage,
    /// Rendered thumbnails and view renders (8-bit) in memory.
    pub rendered: Usage,
    /// Sum of the above.
    pub engine_bytes: usize,
    pub gpu: GpuUsage,
    /// The memory budget (see [`budget`]) and the engine caches' share of it.
    pub budget: usize,
    pub cache_budget: usize,
    /// Working memory of decodes / probes in flight and its limit ([`work_gate`]).
    pub work_bytes: usize,
    pub work_limit: usize,
    /// Live/peak heap when the binary counts allocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heap: Option<HeapUsage>,
}

static HEAP: OnceLock<fn() -> HeapUsage> = OnceLock::new();
static RELEASE: OnceLock<fn()> = OnceLock::new();

/// Install a hook that hands memory the allocator keeps after frees back to the system (first
/// call wins). The macOS allocator keeps freed large blocks resident (reusable, but counted in
/// the resident size) until the system runs short; the apps install
/// `malloc_zone_pressure_relief` so the resident size follows what is actually held.
pub fn set_release_hook(f: fn()) {
    let _ = RELEASE.set(f);
}

/// Return freed memory to the system now (after large temporaries were dropped: a decode, an
/// import). No-op without a hook.
pub fn release() {
    if let Some(f) = RELEASE.get() {
        f();
    }
}

/// Install the heap counter of an instrumented binary (first call wins).
pub fn set_heap_stats(f: fn() -> HeapUsage) {
    let _ = HEAP.set(f);
}

/// Live/peak heap bytes, when the binary installed a counter.
pub fn heap_stats() -> Option<HeapUsage> {
    HEAP.get().map(|f| f())
}

/// The GPU renderer's device buffers.
pub fn gpu_usage() -> GpuUsage {
    let g = lightcraft_gpu::memory();
    GpuUsage { allocated: g.allocated, pooled: g.pooled, retired: g.retired }
}

impl crate::Session {
    /// What the engine's caches hold now.
    pub fn memory_report(&self) -> MemoryReport {
        let (thumb_sources, preview_sources, full_source) = self.media.usage();
        let (n, b) = self.media.rendered.mem_usage();
        let rendered = Usage::new(n, b);
        MemoryReport {
            thumb_sources,
            preview_sources,
            full_source,
            rendered,
            engine_bytes: thumb_sources.bytes + preview_sources.bytes + full_source.bytes + rendered.bytes,
            gpu: gpu_usage(),
            budget: budget(),
            cache_budget: self.media.budget(),
            work_bytes: work_gate().usage().0,
            work_limit: work_gate().usage().1,
            heap: heap_stats(),
        }
    }

    /// Set the memory budget (bytes; process-wide) and apply this session's cache share.
    pub fn set_memory_budget(&mut self, bytes: usize) -> usize {
        let b = set_budget(bytes);
        self.media.set_budget(cache_share(b));
        b
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const GB: usize = 1 << 30;

    /// A host that injects the device's memory (Android, where the engine can't read it) turns on
    /// export's and merge's fit checks: with 4 GB injected an export enlarged far past the photo is
    /// refused, where an unknown amount lets it through to fail allocating. Local cells only: the
    /// process-wide value stays untouched for the other tests.
    #[test]
    fn injected_total_ram_drives_fit_checks() {
        let cell = OnceLock::new();
        assert_eq!(total_ram_in(&cell, || None), None, "nothing injected, nothing probed");
        assert_eq!(set_once_in(&cell, 4 * GB), Ok(()));
        assert_eq!(set_once_in(&cell, 4 * GB), Ok(()), "the same value again");
        assert_eq!(set_once_in(&cell, 8 * GB), Err(4 * GB), "the first value stays");
        let ram = total_ram_in(&cell, || Some(64 * GB));
        assert_eq!(ram, Some(4 * GB), "the injected value wins over the platform probe");
        let why = crate::export::too_big_for_memory((27713, 18475), 24_000_000, 20 * GB, ram).unwrap();
        assert!(why.contains("more than this computer has (4 GB)"), "{why}");
        assert_eq!(crate::export::too_big_for_memory((27713, 18475), 24_000_000, 20 * GB, total_ram_in(&OnceLock::new(), || None)), None);
        let pano = lightcraft_merge::PanoOptions::default();
        let fitted = crate::merge::fit_in_memory(&pano, 100e6, ram);
        assert!(fitted.max_megapixels < crate::merge::fit_in_memory(&pano, 100e6, Some(64 * GB)).max_megapixels);
    }

    /// "Automatic" is the host's budget when it gave one (Android's phone budget), not a quarter
    /// of the RAM it also injected; `LIGHTCRAFT_MEMORY_MB` still wins. Local values only.
    #[test]
    fn injected_default_budget_is_automatic() {
        let mib = 1usize << 20;
        let ram = || Some(8 * GB);
        assert_eq!(default_budget_from(None, None, ram), 1536 * mib, "desktop: a quarter of RAM, at most 1.5 GiB");
        assert_eq!(default_budget_from(None, None, || Some(2 * GB)), 512 * mib);
        assert_eq!(default_budget_from(None, Some(768 * mib), ram), 768 * mib, "the host's budget wins over RAM / 4");
        assert_eq!(default_budget_from(Some(" 2048 "), Some(768 * mib), ram), 2048 * mib, "the variable wins");
        assert_eq!(default_budget_from(Some("12"), Some(768 * mib), ram), 768 * mib, "below 64 MB is ignored");
        let cell = OnceLock::new();
        assert_eq!(set_once_in(&cell, 512 * mib), Ok(()));
        assert_eq!(set_once_in(&cell, 768 * mib), Err(512 * mib), "set once");
        assert_eq!(default_budget_from(None, cell.get().copied(), ram), 512 * mib);
    }
}
