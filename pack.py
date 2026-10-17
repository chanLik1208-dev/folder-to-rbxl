#!/usr/bin/env python3
"""
pack.py — folder-to-rbxl: re-inject edited scripts into a copy of the original
Roblox place/model and write a new .rbxl/.rbxlx/.rbxm/.rbxmx.

This is the reverse of rbxl-to-folder's `extract.py`. Extraction is deliberately
lossy — it only pulls out script `Source` plus the hierarchy — so a faithful
rebuild works by COPYING the original file's full DOM and overwriting only the
script sources from the edited folder tree. Geometry, every non-script property,
non-script instances, and the Disabled flag all ride along untouched.

Because of that, in fill mode you MUST supply the original file the folder was
extracted from.

NEW MODE (--new): build a FRESH place purely from a src/ folder tree — no
original needed. For greenfield projects where every script is new. The tree
follows the same Rojo-style conventions the extractor emits (*.server.lua →
Script, *.client.lua → LocalScript, *.lua → ModuleScript, init.* for
script-with-children); top-level dirs map to Roblox services by name, and a
baseline service set is always created so Studio opens the place cleanly.

Usage:
    python pack.py <extracted_dir> --original <orig.rbxl> [-o OUTPUT]   # fill
    python pack.py <project_dir> --new [-o OUTPUT]                      # new
    python pack.py <extracted_dir> --original <orig.rbxl> --inject spec.json
        # fill, then add new scripts / insert .rbxm(x) models (see README → Inject)

Examples:
    python pack.py ./MyGame_extracted --original MyGame.rbxl
    python pack.py ./MyGame_extracted --original MyGame.rbxl -o MyGame.patched.rbxl
    python pack.py ./my-new-game --new -o MyNewGame.rbxl

In new mode <project_dir>/src is used as the tree root when it exists,
otherwise <project_dir> itself. manifest.json is not required.

Without -o, fill mode outputs <extracted_dir>_packed<ext of original>;
new mode outputs <project_dir>_new.rbxl. Output format follows the output
extension in new mode (.rbxlx/.rbxmx = XML, anything else = binary).

Requires Rust (`cargo`); the engine is built with `cargo build --release` on
first use. Writing a faithful binary .rbxl needs the rbx-dom crates, so unlike
extract.py there is no pure-Python fallback for packing.
"""

import argparse
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RUST_DIR = os.path.join(HERE, "rust")


def rust_exe_path():
    name = "pack-engine.exe" if os.name == "nt" else "pack-engine"
    return os.path.join(RUST_DIR, "target", "release", name)


def ensure_rust_built():
    exe = rust_exe_path()
    if os.path.exists(exe):
        return exe
    if shutil.which("cargo") is None:
        sys.exit("error: cargo (Rust) not found. Packing a faithful .rbxl "
                 "requires the rbx-dom crates; install Rust from https://rustup.rs")
    print("[pack] building Rust engine (first run, may take a minute)...",
          file=sys.stderr)
    subprocess.run(
        ["cargo", "build", "--release"],
        cwd=RUST_DIR, check=True,
        stdout=sys.stderr, stderr=sys.stderr,
    )
    if not os.path.exists(exe):
        sys.exit("error: cargo build finished but binary missing")
    return exe


def main():
    ap = argparse.ArgumentParser(
        description="Fill mode: re-inject edited scripts into a copy of the "
                    "original Roblox file. New mode (--new): build a fresh "
                    "place from a src/ folder tree, no original needed.")
    ap.add_argument("extracted_dir", metavar="dir",
                    help="fill mode: the folder produced by extract.py "
                         "(contains manifest.json and src/); new mode: the "
                         "project dir (its src/ is used when present)")
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--original",
                      help="the original .rbxl/.rbxlx/.rbxm/.rbxmx the folder "
                           "was extracted from (copied, then scripts filled in)")
    mode.add_argument("--new", action="store_true",
                      help="build a fresh place purely from the folder tree")
    ap.add_argument("-o", "--out",
                    help="output path (default: <dir>_packed<ext> in fill "
                         "mode, <dir>_new.rbxl in new mode)")
    ap.add_argument("--inject", metavar="SPEC_JSON",
                    help="fill mode only: after filling, add new scripts and "
                         "insert .rbxm(x) models described by this JSON spec "
                         "(see README → Inject)")
    args = ap.parse_args()
    if args.inject and args.new:
        sys.exit("error: --inject only works in fill mode (--original)")
    if args.inject and not os.path.isfile(args.inject):
        sys.exit(f"error: no such file: {args.inject}")

    if not os.path.isdir(args.extracted_dir):
        sys.exit(f"error: no such directory: {args.extracted_dir}")
    base = os.path.normpath(args.extracted_dir).rstrip(os.sep)

    if args.new:
        src_root = os.path.join(args.extracted_dir, "src")
        if not os.path.isdir(src_root):
            src_root = args.extracted_dir
        out = args.out or (base + "_new.rbxl")
        engine_args = ["--new", src_root, out]
    else:
        manifest = os.path.join(args.extracted_dir, "manifest.json")
        if not os.path.isfile(manifest):
            sys.exit(f"error: {manifest} not found — is this an extract.py "
                     f"output folder?")
        if not os.path.isfile(args.original):
            sys.exit(f"error: no such file: {args.original}")
        ext = os.path.splitext(args.original)[1] or ".rbxl"
        out = args.out or (base + "_packed" + ext)
        engine_args = [args.original, args.extracted_dir, out]
        if args.inject:
            engine_args += ["--inject", args.inject]

    exe = ensure_rust_built()
    try:
        subprocess.run(
            [exe] + engine_args,
            check=True, stderr=sys.stderr, stdout=sys.stderr,
        )
    except subprocess.CalledProcessError as e:
        sys.exit(f"error: pack-engine failed (exit {e.returncode})")

    print(f"[pack] output -> {out}")


if __name__ == "__main__":
    main()
