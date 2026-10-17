//! `cargo xtask android <step>`: the Android port's tooling (docs/android-port.md). One module per
//! step; this file only dispatches.

mod check;
mod elf;
mod manifest;
mod sdk;
mod so;

use crate::StepOutcome;

pub use check::check;

const USAGE: &str = "usage: cargo xtask android <step>\n\nsteps:\n  check   clippy -D warnings for every crate the APK links, for aarch64-linux-android\n  so      link liblightcraft_android.so with the NDK [--abi arm64-v8a|x86_64]… and check it (needs the NDK)\n  manifest  fill packaging/android/AndroidManifest.xml into target/android/ [--release]";

/// `cargo xtask android <step> …`.
pub fn run(args: &[&str]) -> Result<(), String> {
    match args.first().copied() {
        Some("check") => match check()? {
            StepOutcome::Ran => Ok(()),
            StepOutcome::Skipped(why) => {
                println!("android check skipped: {why}");
                Ok(())
            }
        },
        Some("so") => so::run(args.get(1..).unwrap_or_default()),
        Some("manifest") => manifest::run(args.get(1..).unwrap_or_default()),
        Some(other) => Err(format!("android: unknown step `{other}`\n\n{USAGE}")),
        None => Err(format!("android: needs a step\n\n{USAGE}")),
    }
}
