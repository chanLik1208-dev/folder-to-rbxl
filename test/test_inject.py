#!/usr/bin/env python3
"""Test for pack.py --inject (fill mode add-on).

Uses test/fixture.rbxlx + test/fx_extracted (the same pair as the fill-mode
regression) and test/inject_fixture/:

  * creates a new ModuleScript            ReplicatedStorage.NewMod
  * creates a Script under BOTH same-named folders   ReplicatedStorage.Dup.Added  (fan-out)
  * replaces an existing script's Source  ReplicatedStorage.Util.Sub
  * inserts a model (Folder Kit › StringValue + ObjectValue pointing at it)
    under both Dup folders and under Workspace; a second spec entry for
    Workspace must be skipped (same-named child already there)
  * Ref properties inside the inserted model must point at the CLONED
    StringValue, not be null
  * error cases: missing parent / class mismatch exit non-zero

Run:  python test/test_inject.py
"""

import json
import os
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
PACK = os.path.join(HERE, "..", "pack.py")
EXTRACT = os.path.join(HERE, "..", "..", "rbxl-to-folder", "extract.py")
ORIGINAL = os.path.join(HERE, "fixture.rbxlx")
EXTRACTED = os.path.join(HERE, "fx_extracted")
FX = os.path.join(HERE, "inject_fixture")

failures = []


def check(cond, msg):
    print(f"  [{'ok' if cond else 'FAIL'}] {msg}")
    if not cond:
        failures.append(msg)


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def write_spec(tmp, spec, name="spec.json"):
    # file paths in the spec are relative to the spec file → point at inject_fixture
    path = os.path.join(FX, name)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(spec, f)
    return path


GOOD_SPEC = {
    "scripts": [
        {"path": ["ReplicatedStorage", "NewMod"], "class": "ModuleScript", "file": "NewMod.lua"},
        {"path": ["ReplicatedStorage", "Dup", "Added"], "class": "Script", "file": "Added.server.lua"},
        {"path": ["ReplicatedStorage", "Util", "Sub"], "class": "ModuleScript", "file": "SubReplaced.lua"},
    ],
    "models": [
        {"parent": ["ReplicatedStorage", "Dup"], "file": "Kit.rbxmx"},
        {"parent": ["Workspace"], "file": "Kit.rbxmx"},
        {"parent": ["Workspace"], "file": "Kit.rbxmx"},
    ],
}


def read(p):
    with open(p, "rb") as f:
        return f.read()


def main():
    tmp = tempfile.mkdtemp(prefix="inject_test_")
    spec = write_spec(tmp, GOOD_SPEC)
    try:
        for ext in (".rbxlx", ".rbxl"):
            print(f"== inject → {ext}")
            out = os.path.join(tmp, "out" + ext)
            r = run([sys.executable, PACK, EXTRACTED, "--original", ORIGINAL, "--inject", spec, "-o", out])
            check(r.returncode == 0, f"pack --inject exits 0 ({ext})")
            if r.returncode != 0:
                print(r.stdout, r.stderr)
                continue
            check("created 3 script(s), replaced 1, inserted 3 model item(s), skipped 1 existing" in r.stderr,
                  "log reports created 3 / replaced 1 / inserted 3 / skipped 1")

            ex = os.path.join(tmp, "ex" + ext.replace(".", "_"))
            r2 = run([sys.executable, EXTRACT, out, "-o", ex])
            check(r2.returncode == 0, "re-extract succeeds")
            src = os.path.join(ex, "src")
            check(read(os.path.join(src, "ReplicatedStorage", "NewMod.lua")) == read(os.path.join(FX, "NewMod.lua")),
                  "NewMod created with byte-identical source")
            check(read(os.path.join(src, "ReplicatedStorage", "Util", "Sub.lua")) == read(os.path.join(FX, "SubReplaced.lua")),
                  "Util.Sub source replaced")
            tree = open(os.path.join(ex, "tree.txt"), encoding="utf-8").read()
            check(tree.count("Added [Script]") == 2, "Added created under BOTH Dup folders (fan-out)")
            check(tree.count("Kit [Folder]") == 3, "Kit inserted under both Dup folders + Workspace, second Workspace entry skipped")
            check(tree.count("Twin [Script]") == 2, "original scripts still present")

            if ext == ".rbxlx":
                root = ET.parse(out).getroot()
                ok = True
                for kit in root.iter("Item"):
                    props = kit.find("Properties")
                    if kit.get("class") != "Folder" or props.find("string[@name='Name']").text != "Kit":
                        continue
                    label = next(i for i in kit.findall("Item") if i.get("class") == "StringValue")
                    pointer = next(i for i in kit.findall("Item") if i.get("class") == "ObjectValue")
                    ref = pointer.find("Properties/Ref[@name='Value']").text
                    if ref != label.get("referent"):
                        ok = False
                check(ok, "ObjectValue.Value in every inserted Kit points at its own cloned StringValue")

        print("== error cases")
        bad_parent = write_spec(tmp, {"scripts": [{"path": ["Nope", "X"], "class": "ModuleScript", "file": "NewMod.lua"}]},
                                "spec_bad_parent.json")
        r = run([sys.executable, PACK, EXTRACTED, "--original", ORIGINAL, "--inject", bad_parent,
                 "-o", os.path.join(tmp, "bad.rbxlx")])
        check(r.returncode != 0 and "parent not found" in r.stderr, "missing parent → non-zero exit")

        bad_class = write_spec(tmp, {"scripts": [{"path": ["ReplicatedStorage", "Util", "Sub"], "class": "Script",
                                                  "file": "NewMod.lua"}]}, "spec_bad_class.json")
        r = run([sys.executable, PACK, EXTRACTED, "--original", ORIGINAL, "--inject", bad_class,
                 "-o", os.path.join(tmp, "bad2.rbxlx")])
        check(r.returncode != 0 and "exists as ModuleScript" in r.stderr, "class mismatch → non-zero exit")

        r = run([sys.executable, PACK, EXTRACTED, "--new", "--inject", spec, "-o", os.path.join(tmp, "x.rbxl")])
        check(r.returncode != 0, "--inject with --new is rejected")

        print("== regression: fill mode without --inject")
        r = run([sys.executable, PACK, EXTRACTED, "--original", ORIGINAL, "-o", os.path.join(tmp, "plain.rbxlx")])
        check(r.returncode == 0 and "inject:" not in r.stderr, "plain fill mode unchanged (no inject step ran)")
    finally:
        for name in ("spec.json", "spec_bad_parent.json", "spec_bad_class.json"):
            p = os.path.join(FX, name)
            if os.path.exists(p):
                os.remove(p)

    print()
    if failures:
        print(f"FAILED: {len(failures)}")
        sys.exit(1)
    print("ALL PASS")


if __name__ == "__main__":
    main()
