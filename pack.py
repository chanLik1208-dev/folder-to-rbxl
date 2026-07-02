#!/usr/bin/env python3
"""
pack.py — folder-to-rbxl: re-inject edited scripts into a copy of the original
Roblox place/model and write a new .rbxl/.rbxlx/.rbxm/.rbxmx.

This is the reverse of rbxl-to-folder's `extract.py`. Extraction is deliberately
lossy — it only pulls out script `Source` plus the hierarchy — so a faithful
rebuild works by COPYING the original file's full DOM and overwriting only the
script sources from the edited folder tree. Geometry, every non-script property,
non-script instances, and the Disabled flag all ride along untouched.

Because of that, you MUST supply the original file the folder was extracted from.

Usage:
    python pack.py <extracted_dir> --original <orig.rbxl> [-o OUTPUT]

Examples:
    python pack.py ./MyGame_extracted --original MyGame.rbxl
    python pack.py ./MyGame_extracted --original MyGame.rbxl -o MyGame.patched.rbxl

Without -o, output goes next to the extracted dir as
    <extracted_dir>_packed<ext of original>

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
        description="Re-inject edited scripts into a copy of the original "
                    "Roblox file and write a new one.")
    ap.add_argument("extracted_dir",
                    help="the folder produced by extract.py (contains "
                         "manifest.json and src/)")
    ap.add_argument("--original", required=True,
                    help="the original .rbxl/.rbxlx/.rbxm/.rbxmx the folder "
                         "was extracted from (copied, then scripts filled in)")
    ap.add_argument("-o", "--out",
                    help="output path (default: <extracted_dir>_packed<ext>)")
    args = ap.parse_args()

    if not os.path.isdir(args.extracted_dir):
        sys.exit(f"error: no such directory: {args.extracted_dir}")
    manifest = os.path.join(args.extracted_dir, "manifest.json")
    if not os.path.isfile(manifest):
        sys.exit(f"error: {manifest} not found — is this an extract.py output "
                 f"folder?")
    if not os.path.isfile(args.original):
        sys.exit(f"error: no such file: {args.original}")

    ext = os.path.splitext(args.original)[1] or ".rbxl"
    outdir = args.out
    if not outdir:
        base = os.path.normpath(args.extracted_dir).rstrip(os.sep)
        outdir = base + "_packed" + ext

    exe = ensure_rust_built()
    try:
        subprocess.run(
            [exe, args.original, args.extracted_dir, outdir],
            check=True, stderr=sys.stderr, stdout=sys.stderr,
        )
    except subprocess.CalledProcessError as e:
        sys.exit(f"error: pack-engine failed (exit {e.returncode})")

    print(f"[pack] output -> {outdir}")


if __name__ == "__main__":
    main()
