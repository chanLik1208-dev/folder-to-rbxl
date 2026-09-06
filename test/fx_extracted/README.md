# Roblox place extraction (for AI reading)

This folder was produced by `extract.py` from a Roblox place/model file.

- `tree.txt` — the FULL hierarchy rendered like the Roblox Studio Explorer:
  a nested tree with ├──/└── branch lines and a `[ClassName]` tag per instance.
  Use this for the big picture. (On huge places this file can be large — it is
  mostly Workspace geometry; the scripts are the useful part, see `src/`.)
- `src/` — a Rojo-style mirror of the hierarchy. Folders are created only
  along paths that contain scripts, so it stays focused on code.
  - `Script`      -> `*.server.lua`
  - `LocalScript` -> `*.client.lua`
  - `ModuleScript`-> `*.lua`
  - A script that also has (script-bearing) children becomes a folder whose
    own code lives in `init.server.lua` / `init.client.lua` / `init.lua`.
- `manifest.json` — machine-readable list of instances with their class,
  whether they are a script, and the script file path (if any).

Note: non-script instances without any script descendants are listed in
`tree.txt` only (no empty folders are created for them).
