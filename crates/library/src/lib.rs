//! The single owner of content on disk: presets and pedals.
//!
//! A [`Library`] is a data directory laid out as
//!
//! ```text
//! <root>/presets/<name>.toml
//! <root>/pedals/<id>.toml
//! <root>/.seeded.toml        (which factory files were already copied, see `seed_from`)
//! ```
//!
//! It validates names (no path traversal), writes atomically (temp file + rename), reads old
//! preset formats through the graph crate's loader, and seeds factory content. Nothing else in
//! the workspace touches these files directly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use openpedal_graph::load;
use openpedal_graph::model::{GraphFile, PedalFile};

/// Name of the seeding state file inside the data dir.
pub const SEEDED_FILE: &str = ".seeded.toml";

/// Accepts only letters, digits, `-` and `_` (no paths, no `..`).
pub fn sanitize(name: &str) -> Result<&str> {
    let ok = !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("invalid name \"{name}\": use only letters, digits, - and _ (at most 64 characters)");
    }
    Ok(name)
}

/// Factory files already copied into the library (file names, e.g. `clean.toml`).
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Seeded {
    #[serde(default)]
    presets: BTreeSet<String>,
    #[serde(default)]
    pedals: BTreeSet<String>,
}

/// Content on disk. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    /// A library in `root` (`root/presets`, `root/pedals`). Folders are created on first write.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn presets_dir(&self) -> PathBuf {
        self.root.join("presets")
    }

    pub fn pedals_dir(&self) -> PathBuf {
        self.root.join("pedals")
    }

    /// Creates `presets/` and `pedals/` if missing.
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [self.presets_dir(), self.pedals_dir()] {
            std::fs::create_dir_all(&dir).with_context(|| format!("cannot create folder {}", dir.display()))?;
        }
        Ok(())
    }

    // ---- presets ----

    /// Preset names, alphabetical.
    pub fn list_presets(&self) -> Vec<String> {
        list(&self.presets_dir())
    }

    /// Reads a preset (v2, or v1 converted to a graph). The graph's name is set to `name`,
    /// because the interface recognizes the preset by its file name.
    pub fn load_preset(&self, name: &str) -> Result<GraphFile> {
        let path = file_path(&self.presets_dir(), name)?;
        let mut graph = load::read_graph(&path)?;
        graph.name = name.to_string();
        Ok(graph)
    }

    /// Writes a preset; the saved graph's name is `name`.
    pub fn save_preset(&self, name: &str, graph: &GraphFile) -> Result<()> {
        let path = file_path(&self.presets_dir(), name)?;
        let graph = GraphFile { name: name.to_string(), ..graph.clone() };
        write_atomic(&path, &toml::to_string_pretty(&graph).context("cannot serialize the preset")?)
    }

    pub fn delete_preset(&self, name: &str) -> Result<()> {
        delete(&file_path(&self.presets_dir(), name)?)
    }

    /// The preset to start from: `clean`, else the first one. `None` if there are none; the
    /// inner error is returned when the chosen one cannot be read, so the caller can report it.
    pub fn initial_preset(&self) -> Option<Result<GraphFile>> {
        let names = self.list_presets();
        let name = if names.iter().any(|n| n == "clean") { "clean" } else { names.first()? };
        Some(self.load_preset(name))
    }

    // ---- pedals ----

    /// Pedal ids, alphabetical.
    pub fn list_pedals(&self) -> Vec<String> {
        list(&self.pedals_dir())
    }

    /// All pedals: (id, pedal), sorted by id.
    pub fn load_pedals(&self) -> Result<Vec<(String, PedalFile)>> {
        load::read_pedals(&self.pedals_dir())
    }

    pub fn load_pedal(&self, id: &str) -> Result<PedalFile> {
        let path = file_path(&self.pedals_dir(), id)?;
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("error in pedal {}", path.display()))
    }

    pub fn save_pedal(&self, id: &str, pedal: &PedalFile) -> Result<()> {
        let path = file_path(&self.pedals_dir(), id)?;
        write_atomic(&path, &toml::to_string_pretty(pedal).context("cannot serialize the pedal")?)
    }

    pub fn delete_pedal(&self, id: &str) -> Result<()> {
        delete(&file_path(&self.pedals_dir(), id)?)
    }

    // ---- seeding ----

    /// Copies the factory presets and pedals found in `factory_dir/presets` and
    /// `factory_dir/pedals` that this library has never had. Each factory file is seeded once:
    /// its name is remembered in `.seeded.toml`, so a factory preset the user deleted does not
    /// come back, and an existing file is never overwritten (user edits stay). Factory files
    /// added by a later app version are copied the next time. A missing factory dir is not an error.
    pub fn seed_from(&self, factory_dir: &Path) -> Result<()> {
        let state_path = self.root.join(SEEDED_FILE);
        let mut seeded = match std::fs::read_to_string(&state_path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("error in {}", state_path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Seeded::default(),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", state_path.display())),
        };
        let before = toml::to_string(&seeded)?;
        self.ensure_dirs()?;
        seed_kind(&factory_dir.join("presets"), &self.presets_dir(), &mut seeded.presets)?;
        seed_kind(&factory_dir.join("pedals"), &self.pedals_dir(), &mut seeded.pedals)?;
        let after = toml::to_string(&seeded)?;
        if after != before || !state_path.exists() {
            write_atomic(&state_path, &after)?;
        }
        Ok(())
    }
}

fn seed_kind(from: &Path, to: &Path, seeded: &mut BTreeSet<String>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_toml(p)).collect();
    files.sort();
    for src in files {
        let Some(file) = src.file_name().and_then(|n| n.to_str()).map(String::from) else { continue };
        if seeded.contains(&file) {
            continue;
        }
        let target = to.join(&file);
        if !target.exists() {
            let text = std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
            write_atomic(&target, &text)?;
        }
        seeded.insert(file);
    }
    Ok(())
}

fn is_toml(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some("toml")
}

fn file_path(dir: &Path, name: &str) -> Result<PathBuf> {
    Ok(dir.join(format!("{}.toml", sanitize(name)?)))
}

/// Names of the `.toml` files in the folder, without extension, in alphabetical order.
fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_toml(p))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .collect();
    names.sort();
    names
}

/// Writes to a temporary file next to the target and renames it over the target, so a crash
/// never leaves a half-written file.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create folder {}", dir.display()))?;
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", path.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot write {}", path.display()));
    }
    Ok(())
}

fn delete(path: &Path) -> Result<()> {
    std::fs::remove_file(path).with_context(|| format!("cannot delete {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("openpedal-library-{}-{tag}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn graph() -> GraphFile {
        load::parse_graph("name = \"X\"\n[[block]]\ntype = \"gate\"\n").unwrap()
    }

    fn pedal() -> PedalFile {
        toml::from_str("name = \"Test\"\ncategory = \"drive\"\n").unwrap()
    }

    fn factory(root: &Path, presets: &[&str], pedals: &[&str]) {
        for (dir, names) in [("presets", presets), ("pedals", pedals)] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            for n in names {
                let text = if dir == "presets" { "name = \"F\"\n[[block]]\ntype = \"gate\"\n" } else { "name = \"P\"\ncategory = \"drive\"\n" };
                std::fs::write(root.join(dir).join(format!("{n}.toml")), text).unwrap();
            }
        }
    }

    #[test]
    fn names_are_validated() {
        let root = temp_dir("names");
        let lib = Library::new(&root);
        for bad in ["../x", "a/b", "", "a.b", "..", "x y", "\\x", &"a".repeat(65)] {
            assert!(lib.save_preset(bad, &graph()).is_err(), "{bad:?}");
            assert!(lib.save_pedal(bad, &pedal()).is_err());
            assert!(lib.delete_preset(bad).is_err());
            assert!(lib.delete_pedal(bad).is_err());
            assert!(lib.load_preset(bad).is_err());
            assert!(lib.load_pedal(bad).is_err());
        }
        assert!(lib.list_presets().is_empty());
        assert!(sanitize("my-sound_2").is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preset_roundtrip_list_delete() {
        let root = temp_dir("preset");
        let lib = Library::new(&root);
        lib.save_preset("b", &graph()).unwrap();
        lib.save_preset("a", &graph()).unwrap();
        assert_eq!(lib.list_presets(), ["a", "b"]);
        let g = lib.load_preset("a").unwrap();
        assert_eq!(g, GraphFile { name: "a".into(), ..graph() });
        // no temp file is left behind
        assert_eq!(std::fs::read_dir(lib.presets_dir()).unwrap().count(), 2);
        lib.delete_preset("a").unwrap();
        assert_eq!(lib.list_presets(), ["b"]);
        assert!(lib.delete_preset("a").unwrap_err().to_string().contains("cannot delete"));
        assert!(lib.load_preset("a").unwrap_err().to_string().contains("cannot read"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn v1_preset_is_read_as_a_graph_named_after_the_file() {
        let root = temp_dir("v1");
        let lib = Library::new(&root);
        lib.ensure_dirs().unwrap();
        std::fs::write(lib.presets_dir().join("clean.toml"), "name = \"Clean\"\n[[block]]\ntype = \"gate\"\n").unwrap();
        let g = lib.load_preset("clean").unwrap();
        assert_eq!(g.name, "clean");
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(lib.initial_preset().unwrap().unwrap().name, "clean");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pedal_roundtrip() {
        let root = temp_dir("pedal");
        let lib = Library::new(&root);
        lib.save_pedal("test", &pedal()).unwrap();
        assert_eq!(lib.list_pedals(), ["test"]);
        assert_eq!(lib.load_pedal("test").unwrap(), pedal());
        assert_eq!(lib.load_pedals().unwrap().len(), 1);
        lib.delete_pedal("test").unwrap();
        assert!(lib.list_pedals().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unwritable_root_reports_errors() {
        let root = temp_dir("blocked");
        let blocked = root.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let lib = Library::new(&blocked);
        assert!(lib.save_preset("x", &graph()).unwrap_err().to_string().contains("cannot"));
        assert!(lib.save_pedal("x", &pedal()).is_err());
        assert!(lib.delete_preset("x").is_err());
        assert!(lib.seed_from(&root).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn seeding_copies_once_and_respects_deletes_and_edits() {
        let root = temp_dir("seed");
        let (fac, data) = (root.join("factory"), root.join("data"));
        factory(&fac, &["clean", "crunch"], &["fuzz"]);
        let lib = Library::new(&data);
        lib.seed_from(&fac).unwrap();
        assert_eq!(lib.list_presets(), ["clean", "crunch"]);
        assert_eq!(lib.list_pedals(), ["fuzz"]);
        assert!(data.join(SEEDED_FILE).exists());

        // The user deletes one and edits another; the next start changes neither.
        lib.delete_preset("crunch").unwrap();
        lib.save_preset("clean", &graph()).unwrap();
        let edited = std::fs::read_to_string(lib.presets_dir().join("clean.toml")).unwrap();
        lib.seed_from(&fac).unwrap();
        assert_eq!(lib.list_presets(), ["clean"]);
        assert_eq!(std::fs::read_to_string(lib.presets_dir().join("clean.toml")).unwrap(), edited);

        // A factory file added by a new version is copied; an existing user file is not overwritten.
        factory(&fac, &["new", "extra"], &[]);
        std::fs::write(lib.presets_dir().join("new.toml"), "mine").unwrap();
        lib.seed_from(&fac).unwrap();
        assert_eq!(lib.list_presets(), ["clean", "extra", "new"]);
        assert_eq!(std::fs::read_to_string(lib.presets_dir().join("new.toml")).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn seeding_honours_a_prefilled_state_file() {
        // What the iOS host writes when migrating from the old UserDefaults record.
        let root = temp_dir("migrate");
        let (fac, data) = (root.join("factory"), root.join("data"));
        factory(&fac, &["clean", "crunch"], &[]);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join(SEEDED_FILE), "presets = [\"clean.toml\", \"crunch.toml\"]\n").unwrap();
        let lib = Library::new(&data);
        lib.seed_from(&fac).unwrap();
        assert!(lib.list_presets().is_empty(), "deleted factory presets stay deleted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_factory_dir_is_fine() {
        let root = temp_dir("nofactory");
        let lib = Library::new(root.join("data"));
        lib.seed_from(&root.join("nothing")).unwrap();
        assert!(lib.list_presets().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
