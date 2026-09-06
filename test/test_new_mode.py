#!/usr/bin/env python3
"""Round-trip test for pack.py --new.

1. Build test/new_fixture into a fresh .rbxl AND a fresh .rbxlx.
2. Re-extract each with rbxl-to-folder/extract.py.
3. Assert every fixture script reappears at the same relative path with
   byte-identical source, with the right class, and that the baseline
   services exist in the rebuilt place.
4. Regression: legacy fill mode on test/fixture.rbxlx + test/fx_extracted
   still succeeds.

Run:  python test/test_new_mode.py
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.join(HERE, "..", "pack.py")
EXTRACT = os.path.join(HERE, "..", "..", "rbxl-to-folder", "extract.py")
FIXTURE = os.path.join(HERE, "new_fixture")

# fixture file -> (relative extracted path, class). The extractor normalizes
# .luau to .lua on output.
EXPECTED_SCRIPTS = {
    "ReplicatedStorage/Shared/Mod.lua": ("ReplicatedStorage/Shared/Mod.lua", "ModuleScript"),
    "ReplicatedStorage/Util/init.lua": ("ReplicatedStorage/Util/init.lua", "ModuleScript"),
    "ReplicatedStorage/Util/Sub.lua": ("ReplicatedStorage/Util/Sub.lua", "ModuleScript"),
    "ReplicatedStorage/Luau.luau": ("ReplicatedStorage/Luau.lua", "ModuleScript"),
    "ServerScriptService/Boot.server.lua": ("ServerScriptService/Boot.server.lua", "Script"),
    "StarterPlayer/StarterPlayerScripts/UI.client.lua": (
        "StarterPlayer/StarterPlayerScripts/UI.client.lua", "LocalScript"),
    "Workspace/Zone/Thing.server.lua": ("Workspace/Zone/Thing.server.lua", "Script"),
}

BASELINE_SERVICES = [
    "Workspace", "Players", "Lighting", "ReplicatedFirst", "ReplicatedStorage",
    "ServerScriptService", "ServerStorage", "StarterGui", "StarterPack",
    "StarterPlayer", "Teams", "SoundService", "TextChatService",
]

failures = []


def check(cond, msg):
    tag = "ok" if cond else "FAIL"
    print(f"  [{tag}] {msg}")
    if not cond:
        failures.append(msg)


def run(cmd, **kw):
    r = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        print(r.stdout)
        print(r.stderr)
        raise SystemExit(f"command failed ({r.returncode}): {' '.join(cmd)}")
    return r


def round_trip(ext, tmp):
    out_file = os.path.join(tmp, f"built{ext}")
    print(f"\n=== round trip {ext} ===")
    run([sys.executable, PACK, FIXTURE, "--new", "-o", out_file])
    check(os.path.isfile(out_file), f"built{ext} written")

    extracted = os.path.join(tmp, f"re_extracted{ext.lstrip('.')}")
    run([sys.executable, EXTRACT, out_file, "-o", extracted])

    with open(os.path.join(extracted, "manifest.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    by_path = {}
    for entry in manifest:
        by_path.setdefault(entry["path"], []).append(entry)

    # every fixture script reappears: right class, byte-identical source
    for fixture_rel, (out_rel, klass) in EXPECTED_SCRIPTS.items():
        # instance path = file path minus suffix; init.* files belong to the dir
        base = out_rel
        for suffix in (".server.lua", ".client.lua", ".lua"):
            if base.endswith(suffix):
                base = base[: -len(suffix)]
                break
        if base.endswith("/init"):
            base = base[: -len("/init")]
        entries = by_path.get(base, [])
        script_entries = [e for e in entries if e.get("isScript")]
        check(len(script_entries) == 1,
              f"{base}: exactly one script instance (got {len(script_entries)})")
        if not script_entries:
            continue
        entry = script_entries[0]
        check(entry["class"] == klass, f"{base}: class {entry['class']} == {klass}")
        src_file = os.path.join(extracted, "src", *entry["file"].split("/"))
        with open(os.path.join(FIXTURE, "src", *fixture_rel.split("/")), "rb") as f:
            want = f.read()
        with open(src_file, "rb") as f:
            got = f.read()
        check(got == want, f"{base}: source byte-identical")

    # baseline services all present
    for service in BASELINE_SERVICES:
        entries = by_path.get(service, [])
        check(any(e["class"] == service for e in entries),
              f"service {service} present")

    # StarterPlayerScripts has the right class (not Folder)
    sps = by_path.get("StarterPlayer/StarterPlayerScripts", [])
    check(any(e["class"] == "StarterPlayerScripts" for e in sps),
          "StarterPlayerScripts has real container class")
    # Zone stayed a plain Folder
    zone = by_path.get("Workspace/Zone", [])
    check(any(e["class"] == "Folder" for e in zone), "Workspace/Zone is a Folder")
    # Util is a ModuleScript that has a script child
    check(any(e.get("isScript") for e in by_path.get("ReplicatedStorage/Util/Sub", [])),
          "Util/Sub nested under init-based module")


def regression_fill_mode(tmp):
    print("\n=== regression: legacy fill mode ===")
    out_file = os.path.join(tmp, "fill_regression.rbxlx")
    run([sys.executable, PACK, os.path.join(HERE, "fx_extracted"),
         "--original", os.path.join(HERE, "fixture.rbxlx"), "-o", out_file])
    check(os.path.isfile(out_file), "fill mode still writes output")


def main():
    tmp = tempfile.mkdtemp(prefix="new_mode_test_")
    try:
        round_trip(".rbxl", tmp)
        round_trip(".rbxlx", tmp)
        regression_fill_mode(tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    print()
    if failures:
        print(f"test_new_mode: {len(failures)} FAILURE(S)")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("test_new_mode: ALL PASS")


if __name__ == "__main__":
    main()
