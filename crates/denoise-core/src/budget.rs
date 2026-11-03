//! How big the tiles may be, and how many may run at once, within a memory budget (a phone's is a few hundred MB).
//!
//! The model's working memory grows with the area of its tile (every tensor of a fully convolutional U-Net is
//! `channels × side²`), so a model loaded for one tile tells what any other tile needs. The manifest's tile is the
//! largest and the one preferred; when one tile of it does not fit in the share of the budget the tiles get, the side
//! steps down (a multiple the network's poolings divide) until one does, and the tiles that fit side by side run at
//! once. When even the smallest tile does not fit, the plan is refused with what it needs and what there is: running it
//! anyway is what makes Android's low-memory killer end the app without a word.

/// Bytes per cell of a tile that the tile loop itself holds besides the model: the packed input (4 planes), the
/// answer (3 planes of 2 × 2 pixels per cell) and the previous group's answer while it is blended, as `f32`, rounded
/// up.
pub const LOOP_BYTES_PER_CELL: usize = 128;
/// The smallest tile side tried (cells): the smallest a manifest may give.
pub const MIN_TILE: usize = 64;
/// The smallest overlap (cells) a stepped-down tile keeps.
pub const MIN_OVERLAP: usize = 16;
/// Most tiles at once (what [`crate::run::denoise_bayer`] runs in one group).
pub const MAX_PARALLEL: usize = 4;

/// What a loaded model needs, as the planner sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Need {
    /// The manifest's tile side (cells): the largest one used.
    pub max_tile: usize,
    /// The manifest's overlap (cells).
    pub overlap: usize,
    /// The tile side the model was loaded for (cells) and its working memory for one tile then (bytes).
    pub loaded_tile: usize,
    pub loaded_bytes: usize,
    /// Tile sides must be a multiple of this (the network's poolings): at least 16.
    pub multiple: usize,
}

/// The tiles a photo runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub tile: usize,
    pub overlap: usize,
    /// Tiles at once (1 ..= [`MAX_PARALLEL`]).
    pub parallel: usize,
    /// Working memory of one tile (model and tile loop), bytes.
    pub bytes_per_tile: usize,
}

/// Why no plan fits: the smallest tile needs `needs` bytes and the share of the budget is `share`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLittle {
    pub needs: usize,
    pub share: usize,
}

impl std::fmt::Display for TooLittle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AI Denoise needs {} MB of working memory for its smallest tile, and the memory budget gives it {} MB",
            self.needs.div_ceil(1 << 20),
            self.share >> 20
        )
    }
}

/// Working memory of one tile of side `tile` (cells): the model's, scaled from what it needs at the side it was
/// loaded for, and the tile loop's. Saturates instead of overflowing.
pub fn bytes_at(need: &Need, tile: usize) -> usize {
    let area = (tile as u128).saturating_mul(tile as u128);
    let loaded = (need.loaded_tile as u128).saturating_mul(need.loaded_tile as u128).max(1);
    let model = (need.loaded_bytes as u128).saturating_mul(area) / loaded;
    let tile_loop = area.saturating_mul(LOOP_BYTES_PER_CELL as u128);
    usize::try_from(model.saturating_add(tile_loop)).unwrap_or(usize::MAX)
}

/// The overlap a tile of side `tile` keeps: the manifest's, but at most a quarter of the side (even), and at least
/// [`MIN_OVERLAP`]; always less than half the side.
pub fn overlap_for(overlap: usize, tile: usize) -> usize {
    let quarter = (tile / 4) & !1;
    overlap.min(quarter).max(MIN_OVERLAP).min(tile.div_ceil(2).saturating_sub(1)) & !1
}

/// The tiles for `need` within `share` bytes (the part of the memory budget the tiles get), with at most `want` at
/// once: the largest side that fits one tile, then as many of them as fit side by side.
pub fn plan(need: &Need, share: usize, want: usize) -> Result<Plan, TooLittle> {
    let multiple = need.multiple.max(16);
    // a manifest's tile is at most 2048 (checked when it is read); a larger number here is not one
    let top = need.max_tile.min(2048);
    let largest = top - top % multiple;
    let smallest = MIN_TILE.div_ceil(multiple).saturating_mul(multiple).max(multiple);
    let mut tile = largest;
    while tile >= smallest && tile > 0 {
        let bytes = bytes_at(need, tile);
        if bytes <= share {
            let parallel = (share / bytes.max(1)).clamp(1, want.clamp(1, MAX_PARALLEL));
            let overlap = if tile == need.max_tile { need.overlap } else { overlap_for(need.overlap, tile) };
            return Ok(Plan { tile, overlap, parallel, bytes_per_tile: bytes });
        }
        tile -= multiple;
    }
    Err(TooLittle { needs: bytes_at(need, smallest), share })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1 << 20;

    /// A model that needs 200 MiB at its 512-cell tile.
    fn need() -> Need {
        Need { max_tile: 512, overlap: 64, loaded_tile: 512, loaded_bytes: 200 * MIB, multiple: 16 }
    }

    #[test]
    fn plan_keeps_the_manifest_tile_when_it_fits() {
        let p = plan(&need(), 2048 * MIB, 8).unwrap();
        assert_eq!((p.tile, p.overlap, p.parallel), (512, 64, 4));
        assert_eq!(p.bytes_per_tile, 200 * MIB + 512 * 512 * LOOP_BYTES_PER_CELL);
    }

    #[test]
    fn plan_steps_the_tile_down_and_keeps_a_valid_overlap() {
        // a phone's 192 MiB share: one 512 tile (232 MiB) does not fit
        let p = plan(&need(), 192 * MIB, 4).unwrap();
        assert!(p.tile < 512 && p.tile.is_multiple_of(16) && p.tile >= MIN_TILE, "{p:?}");
        assert!(p.bytes_per_tile <= 192 * MIB);
        assert!(bytes_at(&need(), p.tile + 16) > 192 * MIB, "the largest that fits: {p:?}");
        assert!(p.overlap >= MIN_OVERLAP && p.overlap.is_multiple_of(2) && 2 * p.overlap < p.tile, "{p:?}");
        // a network with five poolings wants multiples of 32
        let p = plan(&Need { multiple: 32, ..need() }, 192 * MIB, 4).unwrap();
        assert!(p.tile.is_multiple_of(32), "{p:?}");
    }

    #[test]
    fn plan_refuses_with_what_it_needs() {
        let e = plan(&need(), 2 * MIB, 4).unwrap_err();
        assert_eq!(e.share, 2 * MIB);
        assert_eq!(e.needs, bytes_at(&need(), 64));
        let text = e.to_string();
        assert!(text.contains("MB of working memory") && text.contains("gives it 2 MB"), "{text}");
    }

    #[test]
    fn hostile_numbers_neither_overflow_nor_loop() {
        let huge = Need { max_tile: usize::MAX, overlap: usize::MAX, loaded_tile: 0, loaded_bytes: usize::MAX, multiple: usize::MAX };
        let _ = plan(&huge, usize::MAX, usize::MAX);
        let _ = plan(&Need { multiple: 16, ..huge }, usize::MAX, usize::MAX);
        assert_eq!(bytes_at(&huge, usize::MAX), usize::MAX);
        let tiny = Need { max_tile: 0, overlap: 0, loaded_tile: 0, loaded_bytes: 0, multiple: 0 };
        assert!(plan(&tiny, 0, 0).is_err());
        for t in [0usize, 1, 2, 63, 64, 512] {
            let o = overlap_for(64, t);
            assert!(t < 4 || 2 * o < t, "{t}: {o}");
        }
    }
}
