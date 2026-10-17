//! Project links (Help menu, About dialog).
//!
//! This is a modified version of LightCraft, so it carries none of the ArtCraft marks
//! (docs/brand/LICENSE-brand.txt, "Forks and modified versions"): the links go to this
//! repository, and the About dialog names the original only in plain text ([`UPSTREAM`]).

/// This app's source repository.
pub const GITHUB: &str = "https://github.com/0x4E52/lightcraft-apk";
/// The user documentation (docs/ in the repository).
pub const HELP: &str = "https://github.com/0x4E52/lightcraft-apk/tree/main/docs";
/// Where feedback and bug reports go.
pub const FEEDBACK: &str = "https://github.com/0x4E52/lightcraft-apk/issues/new";
/// The LightCraft repository this version is based on.
pub const UPSTREAM: &str = "https://github.com/storytold/lightcraft";

/// (UI command id, menu label, URL) for each link, in Help-menu order.
pub const LINKS: &[(&str, &str, &str)] =
    &[("app.help", "LightCraft Help", HELP), ("app.feedback", "Send Feedback…", FEEDBACK), ("app.github", "LightCraft on GitHub", GITHUB)];

/// The URL behind a link command id.
pub fn url_of(cmd: &str) -> Option<&'static str> {
    LINKS.iter().find(|(id, _, _)| *id == cmd).map(|(_, _, u)| *u)
}

/// Open `url` in the user's browser (through the host's `open_url` service).
pub fn open(app: &mut crate::LightcraftApp, url: &str) -> Result<serde_json::Value, String> {
    let open = app.services.open_url.as_mut().ok_or("can't open links here")?;
    open(url)?;
    Ok(serde_json::json!({ "url": url }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_https_and_name_this_repository() {
        for (id, label, url) in LINKS {
            assert!(url.starts_with("https://"), "{id}");
            assert!(url.starts_with(GITHUB), "{id} stays in this repository");
            assert!(!label.is_empty());
            assert_eq!(url_of(id), Some(*url));
        }
        assert!(UPSTREAM.starts_with("https://"));
    }

    /// A modified version must not carry the ArtCraft marks (docs/brand/LICENSE-brand.txt).
    #[test]
    fn no_link_uses_the_artcraft_marks() {
        for (id, label, url) in LINKS {
            assert!(!label.to_lowercase().contains("artcraft") && !url.to_lowercase().contains("artcraft"), "{id}");
        }
        for (id, label, _, _) in crate::menus::UI_COMMANDS {
            assert!(!label.to_lowercase().contains("artcraft"), "{id}: {label}");
        }
    }
}
