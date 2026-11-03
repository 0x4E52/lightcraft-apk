//! The tile side and the tiles at once follow the memory budget (`lightcraft_denoise::budget`): a phone's few hundred
//! MB get smaller tiles than a desktop's, never more than the share, and too little is refused with the numbers.

use lightcraft_denoise::budget::{MIN_TILE, Need, bytes_at, plan};

const MIB: usize = 1 << 20;

/// A model like the real ones: about 230 MiB for one 512-cell tile.
fn need() -> Need {
    Need { max_tile: 512, overlap: 64, loaded_tile: 512, loaded_bytes: 200 * MIB, multiple: 16 }
}

#[test]
fn tile_size_follows_budget() {
    // the shares of the budget the tiles get (a quarter): desktop 1.5 GiB, phones with 768, 512 and 256 MiB
    let shares = [384 * MIB, 192 * MIB, 128 * MIB, 64 * MIB];
    let plans: Vec<_> = shares.iter().map(|&s| plan(&need(), s, 4).unwrap()).collect();
    assert_eq!(plans[0].tile, 512, "a desktop keeps the manifest's tile: {:?}", plans[0]);
    for (p, &share) in plans.iter().zip(&shares) {
        assert!(p.bytes_per_tile.saturating_mul(p.parallel) <= share, "{p:?} within {share}");
        assert!(p.tile >= MIN_TILE && p.tile.is_multiple_of(16) && 2 * p.overlap < p.tile, "{p:?}");
    }
    for w in plans.windows(2) {
        assert!(w[1].tile <= w[0].tile, "less memory, never a larger tile: {:?}", w);
    }
    assert!(plans[3].tile < plans[1].tile && plans[1].tile < 512, "{plans:?}");
    // the same model loaded for a smaller tile plans the same (its memory scales with the area)
    let small = Need { loaded_tile: 256, loaded_bytes: 50 * MIB, ..need() };
    assert_eq!(plan(&small, 192 * MIB, 4), Ok(plans[1]));
    assert_eq!(bytes_at(&small, 512), bytes_at(&need(), 512));
    // more room, more tiles at once (at most four)
    assert_eq!(plan(&need(), 4096 * MIB, 8).unwrap().parallel, 4);
    assert_eq!(plan(&need(), 4096 * MIB, 2).unwrap().parallel, 2);
    // a share smaller than the smallest tile is refused, saying what it needs
    let e = plan(&need(), 2 * MIB, 4).unwrap_err();
    assert!(e.needs > e.share && e.to_string().contains("working memory"), "{e}");
}

/// The pure-Rust runner's working memory for a real network grows with the tile's area, as the plan assumes.
#[cfg(feature = "runtime")]
#[test]
fn tile_size_follows_budget_with_the_cpu_runners_memory() {
    use lightcraft_denoise::manifest::{DenoiserManifest, Domain, Gain};
    use lightcraft_denoise::runtime::CpuRunner;
    let dir = std::env::temp_dir().join(format!("lc-denoise-tile-budget-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("unet.onnx");
    std::fs::write(&path, lightcraft_denoise::synthetic::unet_onnx(64, 8, 2, 3)).unwrap();
    let manifest = |tile: u32| DenoiserManifest {
        id: "unet".into(),
        name: "U-Net".into(),
        version: "1".into(),
        licence: Default::default(),
        source: None,
        sha256: None,
        size_bytes: None,
        provenance: "made up for a test".into(),
        domain: Domain::BayerToRgb,
        tile,
        overlap: 16,
        gain: Gain::None,
    };
    let at = |tile: u32| CpuRunner::load(&path, &manifest(tile)).unwrap().bytes_per_tile();
    let (b128, b256) = (at(128), at(256));
    let ratio = b256 as f64 / b128 as f64;
    assert!((3.5..=4.5).contains(&ratio), "{b128} → {b256}");
    // a plan from the 256 runner's memory picks 128 when only that fits, and that tile loads and runs
    let need = Need { max_tile: 256, overlap: 32, loaded_tile: 256, loaded_bytes: b256, multiple: 16 };
    let p = plan(&need, bytes_at(&need, 128) + 1, 1).unwrap();
    assert_eq!(p.tile, 128, "{p:?}");
    let runner = CpuRunner::load(&path, &manifest(p.tile as u32)).unwrap();
    use lightcraft_denoise::run::TileRunner;
    let out = runner.run(&vec![0.25; 4 * 128 * 128]).unwrap();
    assert_eq!(out.len(), 3 * 256 * 256);
    let _ = std::fs::remove_dir_all(&dir);
}
