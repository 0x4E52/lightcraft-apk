//! `cargo xtask android <step>`: the Android port's tooling (docs/android-port.md). One module per
//! step; this file only dispatches.

mod check;

use crate::StepOutcome;

pub use check::check;

const USAGE: &str =
    "usage: cargo xtask android <step>\n\nsteps:\n  check   clippy -D warnings for every crate the APK links, for aarch64-linux-android";

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
        Some(other) => Err(format!("android: unknown step `{other}`\n\n{USAGE}")),
        None => Err(format!("android: needs a step\n\n{USAGE}")),
    }
}
