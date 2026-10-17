//! pack-engine — two modes:
//!
//! **Fill mode** (original): re-inject edited script Source back into a copy of
//! the original Roblox place/model, then re-serialize. Because extraction is
//! deliberately lossy (only script Source + the hierarchy are pulled out), a
//! faithful rebuild works by *copying the original DOM* and overwriting only the
//! script `Source` properties from the edited file tree. Everything else —
//! geometry, every other property, non-script instances, the Disabled flag —
//! rides along untouched from the original.
//!
//!   pack-engine <original.rbxl|.rbxlx|.rbxm|.rbxmx> <extracted_dir> <output>
//!
//! **New mode**: build a FRESH place purely from a src/ folder tree (no
//! original needed) — for greenfield projects. Folder layout follows the same
//! Rojo-style conventions the extractor emits: `*.server.lua(u)` → Script,
//! `*.client.lua(u)` → LocalScript, `*.lua(u)` → ModuleScript, a dir with an
//! `init.*` file → script-with-children named after the dir, other dirs →
//! Folder. Top-level dirs map to Roblox services by name; a baseline service
//! set is always created so Studio opens the place cleanly.
//!
//!   pack-engine --new <src_dir> <output.rbxl|.rbxlx>
//!
//! **Inject (fill-mode add-on)**: after filling, add instances the original
//! does not have — new scripts, and whole models from `.rbxm(x)` files:
//!
//!   pack-engine <original> <extracted_dir> <output> --inject <spec.json>
//!
//!   {
//!     "scripts": [ { "path": ["ReplicatedStorage", "Mods", "NewModule"],
//!                    "class": "ModuleScript", "file": "src/ReplicatedStorage/Mods/NewModule.lua" } ],
//!     "models":  [ { "parent": ["StarterGui"], "file": "gui/MyGui.rbxmx" } ]
//!   }
//!
//! Paths are instance names from the DataModel root; file paths are relative
//! to the spec file. A path that matches several same-named siblings is
//! applied to EVERY match (the log reports counts). Scripts are created if
//! absent, or have their Source replaced if an instance of the same class
//! already exists. Model items are cloned under every matching parent; an
//! existing child with the same name is left alone (skipped).
//!
//! Fill-mode matching strategy: the extractor emits `manifest.json` whose
//! entries are in pre-order DFS order. We reload the original with the SAME
//! rbx-dom crate, walk it in the same pre-order, collect the script instances
//! in order, and zip them against the manifest's script entries. Order-based
//! matching is robust even when sibling instances share a name (name-path
//! matching would be ambiguous). Each pair is integrity-checked on name + class
//! before its Source is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, InstanceBuilder, WeakDom};
use serde::Deserialize;

fn is_script(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

#[derive(Deserialize)]
struct ManifestEntry {
    path: String,
    class: String,
    #[serde(rename = "isScript")]
    is_script: bool,
    file: Option<String>,
}

/// Pre-order DFS collecting script instance refs in the same order the
/// extractor visited them. Non-script-bearing subtrees contain no scripts, so
/// walking the full tree yields the identical script sequence to the extractor's
/// pruned walk.
fn collect_script_refs(dom: &WeakDom, referent: Ref, out: &mut Vec<Ref>) {
    let inst = match dom.get_by_ref(referent) {
        Some(i) => i,
        None => return,
    };
    if is_script(inst.class.as_str()) {
        out.push(referent);
    }
    // clone child list so we don't hold an immutable borrow across recursion
    let children: Vec<Ref> = inst.children().to_vec();
    for child in children {
        collect_script_refs(dom, child, out);
    }
}

fn die(msg: String) -> ! {
    eprintln!("pack-engine: {msg}");
    exit(1);
}

// ---------------------------------------------------------------------------
// New mode: build a fresh place from a src/ folder tree
// ---------------------------------------------------------------------------

/// Known service names (name == ClassName). Top-level dirs matching these are
/// created as real services rather than Folders.
const SERVICE_CLASSES: &[&str] = &[
    "Workspace", "Players", "Lighting", "ReplicatedFirst", "ReplicatedStorage",
    "ServerScriptService", "ServerStorage", "StarterGui", "StarterPack",
    "StarterPlayer", "Teams", "SoundService", "Chat", "TextChatService",
    "LocalizationService", "TestService", "HttpService", "MaterialService",
];

/// Services always present in a new place (even with no scripts inside) so
/// Studio opens it cleanly. Studio regenerates Camera/Terrain itself.
const BASELINE_SERVICES: &[&str] = &[
    "Workspace", "Players", "Lighting", "ReplicatedFirst", "ReplicatedStorage",
    "ServerScriptService", "ServerStorage", "StarterGui", "StarterPack",
    "StarterPlayer", "Teams", "SoundService", "TextChatService",
];

/// Containers that are real classes, not Folders, when found as dirs in the
/// tree (validated by parent class).
fn special_container_class(dir_name: &str, parent_class: &str) -> Option<&'static str> {
    match (dir_name, parent_class) {
        ("StarterPlayerScripts", "StarterPlayer") => Some("StarterPlayerScripts"),
        ("StarterCharacterScripts", "StarterPlayer") => Some("StarterCharacterScripts"),
        _ => None,
    }
}

/// Script class + instance name from a file name following the extractor's
/// (Rojo's) conventions. Returns None for non-Luau files.
fn script_class_for_file(file_name: &str) -> Option<(&'static str, String)> {
    let suffixes: &[(&str, &str)] = &[
        (".server.lua", "Script"),
        (".server.luau", "Script"),
        (".client.lua", "LocalScript"),
        (".client.luau", "LocalScript"),
        (".lua", "ModuleScript"),
        (".luau", "ModuleScript"),
    ];
    for (suffix, class) in suffixes {
        if let Some(stem) = file_name.strip_suffix(suffix) {
            if !stem.is_empty() {
                return Some((class, stem.to_string()));
            }
        }
    }
    None
}

/// If `dir` holds an init file, return (script class, init file path).
/// Errors out if more than one init file is present.
fn find_init(dir: &Path) -> Option<(&'static str, PathBuf)> {
    let candidates: &[(&str, &str)] = &[
        ("init.server.lua", "Script"),
        ("init.server.luau", "Script"),
        ("init.client.lua", "LocalScript"),
        ("init.client.luau", "LocalScript"),
        ("init.lua", "ModuleScript"),
        ("init.luau", "ModuleScript"),
    ];
    let mut found: Option<(&'static str, PathBuf)> = None;
    for (file, class) in candidates {
        let p = dir.join(file);
        if p.is_file() {
            if found.is_some() {
                die(format!("multiple init.* files in {}", dir.display()));
            }
            found = Some((class, p));
        }
    }
    found
}

fn read_source(path: &Path) -> String {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| die(format!("cannot read {}: {e}", path.display())));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Sorted (dirs, files) listing for deterministic output.
fn sorted_entries(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let rd = fs::read_dir(dir)
        .unwrap_or_else(|e| die(format!("cannot list {}: {e}", dir.display())));
    for entry in rd {
        let entry = entry.unwrap_or_else(|e| die(format!("read_dir error: {e}")));
        let p = entry.path();
        if p.is_dir() {
            dirs.push(p);
        } else {
            files.push(p);
        }
    }
    dirs.sort();
    files.sort();
    (dirs, files)
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Reuse an existing child (name + class match) or insert a new instance.
/// Reuse keeps baseline services' pre-created children (StarterPlayerScripts
/// etc.) from being duplicated by dirs in the tree.
fn get_or_insert_child(dom: &mut WeakDom, parent: Ref, name: &str, class: &str) -> Ref {
    let existing = dom
        .get_by_ref(parent)
        .map(|p| p.children().to_vec())
        .unwrap_or_default();
    for child_ref in existing {
        if let Some(child) = dom.get_by_ref(child_ref) {
            if child.name == name && child.class.as_str() == class {
                return child_ref;
            }
        }
    }
    dom.insert(parent, InstanceBuilder::new(class).with_name(name))
}

/// Recursively map a directory's contents into instances under `parent`.
fn walk_dir(dom: &mut WeakDom, parent: Ref, dir: &Path, skip: Option<&Path>) {
    let (dirs, files) = sorted_entries(dir);

    for file in &files {
        if Some(file.as_path()) == skip {
            continue;
        }
        let name = file_name_of(file);
        match script_class_for_file(&name) {
            Some((class, stem)) => {
                let source = read_source(file);
                dom.insert(
                    parent,
                    InstanceBuilder::new(class)
                        .with_name(stem)
                        .with_property("Source", Variant::String(source)),
                );
            }
            None => {
                eprintln!("pack-engine: warning: skipping non-Luau file {}", file.display());
            }
        }
    }

    for sub in &dirs {
        let dir_name = file_name_of(sub);
        match find_init(sub) {
            Some((class, init_path)) => {
                let source = read_source(&init_path);
                let script_ref = dom.insert(
                    parent,
                    InstanceBuilder::new(class)
                        .with_name(dir_name)
                        .with_property("Source", Variant::String(source)),
                );
                walk_dir(dom, script_ref, sub, Some(&init_path));
            }
            None => {
                let parent_class = dom
                    .get_by_ref(parent)
                    .map(|i| i.class.to_string())
                    .unwrap_or_default();
                let class = special_container_class(&dir_name, &parent_class).unwrap_or("Folder");
                let container = get_or_insert_child(dom, parent, &dir_name, class);
                walk_dir(dom, container, sub, None);
            }
        }
    }
}

fn build_new(src_dir: &Path, output: &Path) {
    if !src_dir.is_dir() {
        die(format!("no such directory: {}", src_dir.display()));
    }

    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();

    // baseline services (always present)
    for class in BASELINE_SERVICES {
        let service = dom.insert(root, InstanceBuilder::new(*class));
        if *class == "StarterPlayer" {
            dom.insert(service, InstanceBuilder::new("StarterPlayerScripts"));
            dom.insert(service, InstanceBuilder::new("StarterCharacterScripts"));
        }
    }

    // top-level entries of src/ map to services by name
    let (dirs, files) = sorted_entries(src_dir);
    for file in &files {
        eprintln!(
            "pack-engine: warning: file at src root has no service parent, skipping {}",
            file.display()
        );
    }
    for sub in &dirs {
        let name = file_name_of(sub);
        let is_service = SERVICE_CLASSES.contains(&name.as_str());
        if !is_service {
            eprintln!(
                "pack-engine: warning: top-level '{name}' is not a known service, \
                 creating a Folder under the DataModel root"
            );
        }
        let class = if is_service { name.as_str() } else { "Folder" };
        let service_ref = get_or_insert_child(&mut dom, root, &name, class);
        walk_dir(&mut dom, service_ref, sub, None);
    }

    let mut script_refs = Vec::new();
    for c in dom.root().children().to_vec() {
        collect_script_refs(&dom, c, &mut script_refs);
    }
    let total = script_refs.len();
    let is_binary = !matches!(
        output.extension().and_then(|e| e.to_str()),
        Some("rbxlx") | Some("rbxmx")
    );
    write_dom(&dom, output, is_binary);
    eprintln!("pack-engine: built new place with {total} script(s)");
    eprintln!("pack-engine: wrote {}", output.display());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 2 && args[1] == "--new" {
        if args.len() != 4 {
            eprintln!("usage: pack-engine --new <src_dir> <output>");
            exit(2);
        }
        build_new(Path::new(&args[2]), Path::new(&args[3]));
        return;
    }

    if args.len() < 4 {
        eprintln!("usage: pack-engine <original> <extracted_dir> <output> [--inject <spec.json>]");
        eprintln!("       pack-engine --new <src_dir> <output>");
        exit(2);
    }
    let original = PathBuf::from(&args[1]);
    let extracted_dir = PathBuf::from(&args[2]);
    let output = PathBuf::from(&args[3]);
    let inject_spec: Option<PathBuf> = match args.get(4).map(String::as_str) {
        None => None,
        Some("--inject") => Some(PathBuf::from(
            args.get(5)
                .unwrap_or_else(|| die("--inject needs a spec.json path".into())),
        )),
        Some(other) => die(format!("unknown argument '{other}'")),
    };

    // --- load + decode original (this becomes the copy we fill in) ---
    let bytes = fs::read(&original)
        .unwrap_or_else(|e| die(format!("cannot read {}: {e}", original.display())));

    let is_binary = bytes.starts_with(b"<roblox!");
    let mut dom: WeakDom = if is_binary {
        rbx_binary::from_reader(bytes.as_slice())
            .unwrap_or_else(|e| die(format!("binary decode failed: {e}")))
    } else {
        rbx_xml::from_reader(
            bytes.as_slice(),
            rbx_xml::DecodeOptions::new()
                .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown),
        )
        .unwrap_or_else(|e| die(format!("xml decode failed: {e}")))
    };

    // --- load manifest, keep only the script entries in order ---
    let manifest_path = extracted_dir.join("manifest.json");
    let manifest_raw = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| die(format!("cannot read {}: {e}", manifest_path.display())));
    let manifest: Vec<ManifestEntry> = serde_json::from_str(&manifest_raw)
        .unwrap_or_else(|e| die(format!("manifest.json parse error: {e}")));
    let script_entries: Vec<&ManifestEntry> =
        manifest.iter().filter(|e| e.is_script).collect();

    // --- collect script refs from the DOM in the same pre-order ---
    let root_children: Vec<Ref> = dom.root().children().to_vec();
    let mut script_refs: Vec<Ref> = Vec::new();
    for c in root_children {
        collect_script_refs(&dom, c, &mut script_refs);
    }

    if script_refs.len() != script_entries.len() {
        die(format!(
            "script count mismatch: DOM has {} script instances but manifest \
             lists {}. The extracted folder does not match this original file.",
            script_refs.len(),
            script_entries.len()
        ));
    }

    let src_root = extracted_dir.join("src");
    let mut injected = 0usize;
    let mut missing: Vec<String> = Vec::new();

    for (referent, entry) in script_refs.iter().zip(script_entries.iter()) {
        // integrity check: last path component == instance name, class matches
        let inst = dom.get_by_ref(*referent).expect("ref just collected");
        let leaf = entry.path.rsplit('/').next().unwrap_or(&entry.path);
        if inst.name != leaf || inst.class.as_str() != entry.class {
            die(format!(
                "traversal mismatch at manifest entry '{}' [{}]: DOM has '{}' \
                 [{}]. Extracted folder is out of sync with this original.",
                entry.path, entry.class, inst.name, inst.class
            ));
        }
        let file_rel = match &entry.file {
            Some(f) => f,
            None => continue, // script entry without a file: nothing to inject
        };
        // join the forward-slash relative path onto src/
        let mut file_path = src_root.clone();
        for part in file_rel.split('/') {
            file_path.push(part);
        }
        if !file_path.exists() {
            missing.push(file_rel.clone());
            continue;
        }
        let new_bytes = fs::read(&file_path)
            .unwrap_or_else(|e| die(format!("cannot read {}: {e}", file_path.display())));

        // preserve the original Source variant type (String vs BinaryString)
        let as_binary = matches!(
            inst.properties.get(&ustr("Source")),
            Some(Variant::BinaryString(_))
        );
        let inst_mut = dom.get_by_ref_mut(*referent).expect("ref just collected");
        let value = if as_binary {
            Variant::BinaryString(new_bytes.into())
        } else {
            Variant::String(String::from_utf8_lossy(&new_bytes).into_owned())
        };
        inst_mut.properties.insert(ustr("Source"), value);
        injected += 1;
    }

    if let Some(spec) = &inject_spec {
        apply_inject(&mut dom, spec);
    }

    // --- serialize the filled copy ---
    // Output format follows the OUTPUT extension (.rbxlx/.rbxmx = XML, else binary),
    // not the input format. Lets fill mode emit rbxmx/rbxlx from a binary original.
    let out_is_binary = !matches!(
        output.extension().and_then(|e| e.to_str()),
        Some("rbxlx") | Some("rbxmx")
    );
    write_dom(&dom, &output, out_is_binary);

    eprintln!(
        "pack-engine: injected {injected} script(s); {} missing file(s){}",
        missing.len(),
        if missing.is_empty() { String::new() } else { format!(": {}", missing.join(", ")) }
    );
    eprintln!("pack-engine: wrote {}", output.display());
}

// ---------------------------------------------------------------------------
// Inject: add new scripts / models after filling
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct InjectSpec {
    #[serde(default)]
    scripts: Vec<InjectScript>,
    #[serde(default)]
    models: Vec<InjectModel>,
}

#[derive(Deserialize)]
struct InjectScript {
    path: Vec<String>,
    class: String,
    file: String,
}

#[derive(Deserialize)]
struct InjectModel {
    parent: Vec<String>,
    file: String,
}

/// Every instance reachable by following `path` (names) from the DataModel
/// root. Same-named siblings fan out, so the result can hold several refs.
fn resolve_all(dom: &WeakDom, path: &[String]) -> Vec<Ref> {
    let mut current = vec![dom.root_ref()];
    for name in path {
        let mut next = Vec::new();
        for r in &current {
            if let Some(inst) = dom.get_by_ref(*r) {
                for c in inst.children() {
                    if let Some(ci) = dom.get_by_ref(*c) {
                        if ci.name == *name {
                            next.push(*c);
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            return next;
        }
        current = next;
    }
    current
}

fn child_named(dom: &WeakDom, parent: Ref, name: &str) -> Option<Ref> {
    dom.get_by_ref(parent)?
        .children()
        .iter()
        .copied()
        .find(|c| dom.get_by_ref(*c).map(|i| i.name == name).unwrap_or(false))
}

fn load_model(path: &Path) -> WeakDom {
    let bytes = fs::read(path).unwrap_or_else(|e| die(format!("cannot read {}: {e}", path.display())));
    if bytes.starts_with(b"<roblox!") {
        rbx_binary::from_reader(bytes.as_slice())
            .unwrap_or_else(|e| die(format!("binary decode failed for {}: {e}", path.display())))
    } else {
        rbx_xml::from_reader(
            bytes.as_slice(),
            rbx_xml::DecodeOptions::new()
                .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown),
        )
        .unwrap_or_else(|e| die(format!("xml decode failed for {}: {e}", path.display())))
    }
}

fn apply_inject(dom: &mut WeakDom, spec_path: &Path) {
    let raw = fs::read_to_string(spec_path)
        .unwrap_or_else(|e| die(format!("cannot read {}: {e}", spec_path.display())));
    let spec: InjectSpec = serde_json::from_str(&raw)
        .unwrap_or_else(|e| die(format!("inject spec parse error: {e}")));
    let base = spec_path.parent().unwrap_or_else(|| Path::new("."));

    let (mut created, mut replaced, mut inserted, mut skipped) = (0usize, 0usize, 0usize, 0usize);

    for s in &spec.scripts {
        if !is_script(&s.class) {
            die(format!("inject: '{}' is not a script class", s.class));
        }
        if s.path.is_empty() {
            die("inject: empty script path".into());
        }
        let source = fs::read(base.join(&s.file))
            .unwrap_or_else(|e| die(format!("cannot read {}: {e}", s.file)));
        let source = String::from_utf8_lossy(&source).into_owned();
        let (parent_path, leaf) = s.path.split_at(s.path.len() - 1);
        let name = &leaf[0];
        let parents = resolve_all(dom, parent_path);
        if parents.is_empty() {
            die(format!("inject: parent not found for {}", s.path.join(".")));
        }
        for parent in parents {
            match child_named(dom, parent, name) {
                Some(existing) => {
                    let inst = dom.get_by_ref_mut(existing).expect("child exists");
                    if inst.class.as_str() != s.class {
                        die(format!(
                            "inject: {} exists as {} (wanted {})",
                            s.path.join("."),
                            inst.class,
                            s.class
                        ));
                    }
                    inst.properties.insert(ustr("Source"), Variant::String(source.clone()));
                    replaced += 1;
                }
                None => {
                    dom.insert(
                        parent,
                        InstanceBuilder::new(s.class.as_str())
                            .with_name(name.as_str())
                            .with_property("Source", Variant::String(source.clone())),
                    );
                    created += 1;
                }
            }
        }
    }

    for m in &spec.models {
        let model = load_model(&base.join(&m.file));
        let items: Vec<Ref> = model.root().children().to_vec();
        let parents = resolve_all(dom, &m.parent);
        if parents.is_empty() {
            die(format!(
                "inject: model parent not found: {} ({})",
                m.parent.join("."),
                m.file
            ));
        }
        for parent in parents {
            for item in &items {
                let item_name = model.get_by_ref(*item).map(|i| i.name.clone()).unwrap_or_default();
                if child_named(dom, parent, &item_name).is_some() {
                    skipped += 1;
                    continue;
                }
                let cloned = model.clone_into_external(*item, dom);
                dom.transfer_within(cloned, parent);
                inserted += 1;
            }
        }
    }

    eprintln!(
        "pack-engine: inject: created {created} script(s), replaced {replaced}, inserted {inserted} model item(s), skipped {skipped} existing"
    );
}

fn write_dom(dom: &WeakDom, output: &Path, is_binary: bool) {
    let file = fs::File::create(output)
        .unwrap_or_else(|e| die(format!("cannot create {}: {e}", output.display())));
    let refs: Vec<Ref> = dom.root().children().to_vec();
    if is_binary {
        rbx_binary::to_writer(file, dom, &refs)
            .unwrap_or_else(|e| die(format!("binary encode failed: {e}")));
    } else {
        rbx_xml::to_writer_default(file, dom, &refs)
            .unwrap_or_else(|e| die(format!("xml encode failed: {e}")));
    }
}
