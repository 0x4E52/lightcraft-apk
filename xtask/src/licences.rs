//! `cargo xtask licences`: the inventory of third-party licences that ships inside the app
//! (About ▸ Licences) and the policy every shipped dependency must meet.
//!
//! The inventory covers the crates the shipped apps link (normal dependencies of [`SHIPPED`], all
//! features, every target), not build tools or test-only crates. Each entry keeps the licence files
//! the crate publishes (LICENSE*, COPYING*, NOTICE*…); a crate that publishes none gets the standard
//! text of the licence it is used under from `assets/licences/spdx/`. Texts are stored once and
//! referenced by index, since most crates share the same Apache-2.0 text.
//!
//! Without `--write` the command checks that the committed inventory is current (part of `ci`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// The committed inventory, read by `crates/ui-egui/src/licences.rs`.
pub const OUTPUT: &str = "assets/licences/third-party.json";
/// Standard licence texts (`<SPDX id>.txt`) for crates that publish no licence file.
const STANDARD_TEXTS: &str = "assets/licences/spdx";

/// The packages that are distributed: their dependencies are what the inventory lists.
const SHIPPED: &[&str] = &["lightcraft", "lightcraft-cli", "lightcraft-web"];

/// Licences a shipped dependency may be used under: permissive licences, and MPL-2.0 (file-level
/// copyleft, which an unmodified dependency meets by naming its source). Nothing from the GPL
/// family: a crate offering GPL only as one alternative is used under another.
const ALLOWED: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-1-Clause",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "BSL-1.0",
    "CC0-1.0",
    "CDLA-Permissive-2.0",
    "IJG",
    "ISC",
    "MIT",
    "MIT-0",
    "MPL-2.0",
    "NCSA",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "Zlib",
];

/// Which alternative to comply with when a crate offers a choice: LightCraft's own licences first.
const PREFERENCE: &[&str] = &["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-3-Clause", "ISC", "Zlib"];

/// File-name prefixes (case-insensitive) of the licence and notice files in a crate's root.
const TEXT_PREFIXES: &[&str] = &["LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE"];

/// A crate the shipped apps link, as `cargo metadata` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    /// The SPDX expression from `Cargo.toml`.
    pub licence: String,
    pub authors: Vec<String>,
    pub repository: Option<String>,
    /// The crate's root directory.
    pub dir: PathBuf,
    /// `license-file` from `Cargo.toml`, when given.
    pub licence_file: Option<PathBuf>,
}

pub fn run(root: &Path, write: bool) -> Result<(), String> {
    let packages = shipped(&full_metadata()?)?;
    let json = render(&packages, &crate_texts, &|id| standard_text(root, id))?;
    let path = root.join(OUTPUT);
    if write {
        std::fs::write(&path, &json).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("licences: wrote {OUTPUT} ({} crates)", packages.len());
        return Ok(());
    }
    let committed = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if committed != json {
        return Err(format!("{OUTPUT} is out of date (dependencies changed): run `cargo xtask licences --write` and commit it"));
    }
    println!("licences: OK, {} third-party crates, all under allowed licences ({OUTPUT} is current)", packages.len());
    Ok(())
}

/// `cargo metadata` with the dependency graph, all features enabled.
fn full_metadata() -> Result<Value, String> {
    let out = crate::cargo().args(["metadata", "--format-version", "1", "--all-features"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

/// The third-party packages reachable from [`SHIPPED`] over normal dependencies, sorted by name
/// and version.
pub fn shipped(meta: &Value) -> Result<Vec<Package>, String> {
    let packages: HashMap<&str, &Value> =
        meta["packages"].as_array().ok_or("metadata: no packages")?.iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let workspace: HashSet<&str> =
        meta["workspace_members"].as_array().ok_or("metadata: no workspace members")?.iter().filter_map(Value::as_str).collect();
    let nodes: HashMap<&str, &Value> =
        meta["resolve"]["nodes"].as_array().ok_or("metadata: no dependency graph")?.iter().filter_map(|n| Some((n["id"].as_str()?, n))).collect();

    let mut stack = Vec::new();
    for name in SHIPPED {
        let id = workspace
            .iter()
            .find(|id| packages.get(**id).is_some_and(|p| p["name"] == *name))
            .ok_or(format!("shipped package `{name}` is not in the workspace"))?;
        stack.push(*id);
    }
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let deps = nodes.get(id).and_then(|n| n["deps"].as_array()).map(Vec::as_slice).unwrap_or_default();
        for dep in deps {
            let normal = dep["dep_kinds"].as_array().is_some_and(|kinds| kinds.iter().any(|k| k["kind"].is_null()));
            if let (true, Some(pkg)) = (normal, dep["pkg"].as_str()) {
                stack.push(pkg);
            }
        }
    }

    let mut out = Vec::new();
    for id in seen.into_iter().filter(|id| !workspace.contains(id)) {
        let p = packages.get(id).ok_or(format!("metadata: no package `{id}`"))?;
        let field = |key: &str| p[key].as_str().map(str::to_string);
        let name = field("name").ok_or(format!("metadata: `{id}` has no name"))?;
        let manifest = field("manifest_path").ok_or(format!("metadata: `{name}` has no manifest path"))?;
        let dir = Path::new(&manifest).parent().ok_or(format!("metadata: `{name}`: bad manifest path"))?.to_path_buf();
        out.push(Package {
            licence: field("license").ok_or(format!("`{name}` declares no licence in its Cargo.toml"))?,
            version: field("version").unwrap_or_default(),
            authors: p["authors"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            repository: field("repository"),
            licence_file: field("license_file").map(|f| dir.join(f)),
            dir,
            name,
        });
    }
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(out)
}

/// The licence ids `licence` (an SPDX expression) is used under, or why it can't be shipped.
pub fn comply_with(licence: &str) -> Result<Vec<String>, String> {
    let expr = crate::spdx::parse(licence)?;
    expr.choose(&|id| ALLOWED.contains(&id), PREFERENCE).ok_or(format!("`{licence}` offers no licence LightCraft may ship under"))
}

/// The licence and notice files a crate publishes, as (file name, text), sorted by name.
fn crate_texts(p: &Package) -> Result<Vec<(String, String)>, String> {
    let read = |path: &Path| std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{}: {e}", path.display()));
    let mut files = BTreeMap::new();
    let entries = std::fs::read_dir(&p.dir).map_err(|e| format!("{}: {e}", p.dir.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let upper = name.to_ascii_uppercase();
        if TEXT_PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) && entry.path().is_file() {
            files.insert(name, read(&entry.path())?);
        }
    }
    if let Some(path) = &p.licence_file {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let std::collections::btree_map::Entry::Vacant(slot) = files.entry(name) {
            slot.insert(read(path)?);
        }
    }
    Ok(files.into_iter().collect())
}

fn standard_text(root: &Path, id: &str) -> Result<String, String> {
    let path = root.join(STANDARD_TEXTS).join(format!("{id}.txt"));
    std::fs::read_to_string(&path).map_err(|_| format!("no standard text for `{id}`: add {}", path.strip_prefix(root).unwrap_or(&path).display()))
}

/// Line endings and trailing blanks don't make a different licence.
fn normalize(text: &str) -> String {
    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// The inventory as pretty JSON: `packages` (name, version, the declared licence, the licences
/// complied with, authors, repository, indices into `texts`, and whether those are the crate's own
/// files or standard texts) and `texts` (title, text). Every policy violation is reported at once.
pub fn render(
    packages: &[Package],
    texts_of: &dyn Fn(&Package) -> Result<Vec<(String, String)>, String>,
    standard: &dyn Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut texts: Vec<Value> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut intern = |title: String, text: &str| {
        let text = normalize(text);
        *index.entry(text.clone()).or_insert_with(|| {
            texts.push(json!({ "title": title, "text": text }));
            texts.len() - 1
        })
    };
    let (mut entries, mut problems) = (Vec::new(), Vec::new());
    for p in packages {
        let ids = match comply_with(&p.licence) {
            Ok(ids) => ids,
            Err(e) => {
                problems.push(format!("{} {}: {e}", p.name, p.version));
                continue;
            }
        };
        let mut own = texts_of(p)?;
        own.retain(|(_, text)| !text.trim().is_empty());
        let standard_texts = own.is_empty();
        let refs: Vec<usize> = if standard_texts {
            let mut refs = Vec::new();
            for id in &ids {
                match standard(id) {
                    Ok(text) => refs.push(intern(format!("{id} (standard text)"), &text)),
                    Err(e) => problems.push(format!("{} {} publishes no licence file and {e}", p.name, p.version)),
                }
            }
            refs
        } else {
            own.iter().map(|(title, text)| intern(title.clone(), text)).collect()
        };
        entries.push(json!({
            "name": p.name,
            "version": p.version,
            "licence": p.licence,
            "complies_with": ids,
            "authors": p.authors,
            "repository": p.repository,
            "texts": refs,
            "standard_texts": standard_texts,
        }));
    }
    if !problems.is_empty() {
        return Err(format!("licence policy ({} problem(s)):\n  {}", problems.len(), problems.join("\n  ")));
    }
    let doc = json!({
        "generated_by": "cargo xtask licences --write",
        "packages": entries,
        "texts": texts,
    });
    serde_json::to_string_pretty(&doc).map(|s| s + "\n").map_err(|e| format!("licences: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, licence: &str) -> Package {
        Package {
            name: name.into(),
            version: "1.0.0".into(),
            licence: licence.into(),
            authors: vec!["A. Author".into()],
            repository: None,
            dir: PathBuf::from(name),
            licence_file: None,
        }
    }

    #[test]
    fn copyleft_only_crates_are_refused() {
        assert_eq!(comply_with("MIT OR Apache-2.0").unwrap(), ["MIT"]);
        assert_eq!(comply_with("Apache-2.0 OR GPL-2.0-only").unwrap(), ["Apache-2.0"]);
        assert_eq!(comply_with("MIT OR Apache-2.0 OR LGPL-2.1-or-later").unwrap(), ["MIT"]);
        assert_eq!(comply_with("(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0").unwrap(), ["MIT", "OFL-1.1", "Ubuntu-font-1.0"]);
        for refused in ["GPL-3.0-only", "AGPL-3.0-or-later", "LGPL-2.1-or-later", "MIT AND GPL-2.0-only", "SSPL-1.0", "MIT OR"] {
            assert!(comply_with(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn texts_are_shared_and_standard_texts_fill_gaps() {
        let packages = [package("a", "MIT"), package("b", "Apache-2.0/MIT"), package("c", "MIT")];
        let texts_of = |p: &Package| -> Result<Vec<(String, String)>, String> {
            Ok(match p.name.as_str() {
                "a" => vec![("LICENSE".into(), "Copyright A\r\nMIT terms  \n".into())],
                "b" => vec![("LICENSE-MIT".into(), "Copyright A\nMIT terms".into()), ("NOTICE".into(), "  ".into())],
                _ => vec![],
            })
        };
        let standard = |id: &str| -> Result<String, String> { Ok(format!("standard {id}")) };
        let doc: Value = serde_json::from_str(&render(&packages, &texts_of, &standard).unwrap()).unwrap();
        let texts = doc["texts"].as_array().unwrap();
        assert_eq!(texts.len(), 2, "a and b share one text after normalizing; the blank NOTICE is dropped: {texts:?}");
        assert_eq!(texts[0]["text"], "Copyright A\nMIT terms");
        assert_eq!(texts[1]["title"], "MIT (standard text)");
        let p = &doc["packages"];
        assert_eq!(p[0]["texts"], json!([0]));
        assert_eq!(p[1]["texts"], json!([0]));
        assert_eq!(p[1]["complies_with"], json!(["MIT"]));
        assert_eq!((p[2]["texts"].clone(), p[2]["standard_texts"].clone()), (json!([1]), json!(true)));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let packages = [package("gpl", "GPL-3.0-only"), package("fine", "MIT"), package("bare", "Zlib")];
        let none = |_: &Package| -> Result<Vec<(String, String)>, String> { Ok(vec![]) };
        let only_mit = |id: &str| if id == "MIT" { Ok("MIT".to_string()) } else { Err(format!("no standard text for `{id}`")) };
        let e = render(&packages, &none, &only_mit).unwrap_err();
        assert!(e.contains("2 problem(s)") && e.contains("gpl 1.0.0") && e.contains("bare 1.0.0 publishes no licence file"), "{e}");
    }

    #[test]
    fn only_normal_dependencies_of_shipped_apps_count() {
        let pkg = |id: &str, name: &str| json!({"id": id, "name": name, "version": "1.0.0", "license": "MIT", "manifest_path": format!("/r/{name}/Cargo.toml")});
        let dep = |id: &str, kind: Value| json!({"pkg": id, "dep_kinds": [{"kind": kind}]});
        let mut packages = vec![pkg("app", "lightcraft"), pkg("lib", "used"), pkg("dev", "dev-only"), pkg("bld", "build-only"), pkg("deep", "deep")];
        packages.extend(SHIPPED.iter().skip(1).map(|n| pkg(n, n)));
        let mut members = vec!["app".to_string()];
        members.extend(SHIPPED.iter().skip(1).map(|n| n.to_string()));
        let meta = json!({
            "packages": packages,
            "workspace_members": members,
            "resolve": {"nodes": [
                {"id": "app", "deps": [dep("lib", Value::Null), dep("dev", json!("dev")), dep("bld", json!("build"))]},
                {"id": "lib", "deps": [dep("deep", Value::Null)]},
            ]},
        });
        let names: Vec<String> = shipped(&meta).unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["deep", "used"]);
    }
}
