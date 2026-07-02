//! pack-engine — re-inject edited script Source back into a copy of the
//! original Roblox place/model, then re-serialize.
//!
//! This is the reverse of rbxl-to-folder's `extract.py`. Because extraction is
//! deliberately lossy (only script Source + the hierarchy are pulled out), a
//! faithful rebuild works by *copying the original DOM* and overwriting only the
//! script `Source` properties from the edited file tree. Everything else —
//! geometry, every other property, non-script instances, the Disabled flag —
//! rides along untouched from the original.
//!
//! Usage:
//!   pack-engine <original.rbxl|.rbxlx|.rbxm|.rbxmx> <extracted_dir> <output>
//!
//! Matching strategy: the extractor emits `manifest.json` whose entries are in
//! pre-order DFS order. We reload the original with the SAME rbx-dom crate, walk
//! it in the same pre-order, collect the script instances in order, and zip them
//! against the manifest's script entries. Order-based matching is robust even
//! when sibling instances share a name (name-path matching would be ambiguous).
//! Each pair is integrity-checked on name + class before its Source is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use rbx_dom_weak::WeakDom;
use rbx_types::{Ref, Variant};
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
    if is_script(&inst.class) {
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: pack-engine <original> <extracted_dir> <output>");
        exit(2);
    }
    let original = PathBuf::from(&args[1]);
    let extracted_dir = PathBuf::from(&args[2]);
    let output = PathBuf::from(&args[3]);

    // --- load + decode original (this becomes the copy we fill in) ---
    let bytes = fs::read(&original)
        .unwrap_or_else(|e| die(format!("cannot read {}: {e}", original.display())));

    let is_binary = bytes.starts_with(b"<roblox!");
    let mut dom: WeakDom = if is_binary {
        rbx_binary::from_reader(bytes.as_slice())
            .unwrap_or_else(|e| die(format!("binary decode failed: {e}")))
    } else {
        rbx_xml::from_reader(bytes.as_slice(), rbx_xml::DecodeOptions::new())
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
        if inst.name != leaf || inst.class != entry.class {
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
            inst.properties.get("Source"),
            Some(Variant::BinaryString(_))
        );
        let inst_mut = dom.get_by_ref_mut(*referent).expect("ref just collected");
        let value = if as_binary {
            Variant::BinaryString(new_bytes.into())
        } else {
            Variant::String(String::from_utf8_lossy(&new_bytes).into_owned())
        };
        inst_mut.properties.insert("Source".into(), value);
        injected += 1;
    }

    // --- serialize the filled copy ---
    write_dom(&dom, &output, is_binary);

    eprintln!(
        "pack-engine: injected {injected} script(s); {} missing file(s){}",
        missing.len(),
        if missing.is_empty() { String::new() } else { format!(": {}", missing.join(", ")) }
    );
    eprintln!("pack-engine: wrote {}", output.display());
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
