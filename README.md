# folder-to-rbxl

**English** | [中文](#folder-to-rbxl中文)

The reverse of [rbxl-to-folder](../rbxl-to-folder): take a folder tree that was
extracted from a Roblox place/model, re-inject the (edited) script source, and
write a new `.rbxl` / `.rbxlx` / `.rbxm` / `.rbxmx`.

## The key idea: copy-and-fill

Extraction is **deliberately lossy** — it only pulls out script `Source` plus the
hierarchy. Geometry, meshes, and every non-script property are *not* stored in the
folder. So a faithful rebuild is **not** "build a new file from the folder"; it is:

1. **Copy** the original file's full DOM (every instance, every property).
2. **Fill in** only the script `Source` values from the edited folder tree.
3. Re-serialize.

Everything that is not a script's source code — geometry, non-script instances,
the `Disabled` flag, all other properties — rides along untouched from the copy.

**Because of this you must supply the original file** the folder was extracted
from. This tool edits code; it does not reconstruct geometry from nothing.

## Usage

```bash
python pack.py <extracted_dir> --original <orig.rbxl> [-o OUTPUT]
```

Examples:

```bash
python pack.py ./MyGame_extracted --original MyGame.rbxl
python pack.py ./MyGame_extracted --original MyGame.rbxl -o MyGame.patched.rbxl
```

Without `-o`, output goes next to the extracted dir as
`<extracted_dir>_packed<ext-of-original>`. The output format follows the
original (binary in → binary out, XML in → XML out).

## How scripts are matched back

The extractor writes `manifest.json` with one entry per instance in **pre-order
DFS** order. `pack.py` reloads the original with the same rbx-dom crate, walks it
in the same pre-order, collects the script instances in order, and zips them
against the manifest's script entries. Matching is **positional, not by name**,
so it stays correct even when sibling instances share a name (e.g. two `Twin`
scripts in two folders both called `Dup` — a case name-path matching would get
wrong). Each pair is integrity-checked on name + class before its `Source` is
overwritten; a mismatch aborts rather than corrupting.

If a script file is missing from the folder, that instance keeps its original
source (reported as a "missing file"). If the folder's script count doesn't
match the original's, the tool aborts — the folder is out of sync with that
original.

## Requirements

Rust (`cargo`) — the engine (`rust/pack-engine`) is built with
`cargo build --release` on first run. Writing a faithful binary `.rbxl` needs the
[rbx-dom](https://github.com/rojo-rbx/rbx-dom) crates
(`rbx_binary` / `rbx_xml` / `rbx_dom_weak`), so — unlike `extract.py` — there is
**no** pure-Python fallback for packing.

## New mode (`--new`): build a fresh place from a folder tree

For greenfield projects where every script is new (no original to fill), `--new`
builds a **fresh** place purely from a `src/` tree:

```bash
python pack.py <project_dir> --new [-o Game.rbxl]
```

`<project_dir>/src` is used as the tree root when it exists, otherwise
`<project_dir>` itself; `manifest.json` is not needed. The tree follows the same
Rojo-style conventions the extractor emits (`*.server.lua(u)` → `Script`,
`*.client.lua(u)` → `LocalScript`, `*.lua(u)` → `ModuleScript`, a dir with an
`init.*` file → script-with-children named after the dir, other dirs → `Folder`).
Top-level dirs map to Roblox services by name (`ReplicatedStorage`,
`ServerScriptService`, `StarterPlayer/StarterPlayerScripts`, …), and a baseline
service set is always created so Studio opens the place cleanly. Output format
follows the output extension (`.rbxlx`/`.rbxmx` = XML, anything else = binary).

Round-trip test: `python test/test_new_mode.py` (builds the fixture both ways,
re-extracts with rbxl-to-folder, compares sources byte-for-byte, and runs a
fill-mode regression).

## Scope / limitations

- **Fill mode is script source only.** Renaming/adding/deleting instances in the
  folder is not reflected; only the `Source` of scripts already present in the
  original is overwritten. (Add a script in Studio, re-extract, then edit — or
  use `--new` for fully code-defined projects.)
- Fill mode requires the **matching** original. A folder extracted from a
  *different* build of the place will fail the count / integrity check.
- New mode emits scripts/Folders/services only — no geometry or non-default
  properties; anything else must be created at runtime by code.

---

# folder-to-rbxl(中文)

[English](#folder-to-rbxl) | **中文**

[rbxl-to-folder](../rbxl-to-folder) 的反向工具:把「解壓出來的資料夾樹」裡(改過的)
腳本原始碼**填回**,產生新的 `.rbxl` / `.rbxlx` / `.rbxm` / `.rbxmx`。

## 核心作法:複製再填入

解壓是**刻意有損的**——只抽出腳本 `Source` 與階層,幾何、mesh、所有非腳本屬性都**不**
存在資料夾裡。所以忠實的重建**不是**「從資料夾生一個新檔」,而是:

1. **複製**原始檔的完整 DOM(每個 instance、每個屬性)。
2. 只把腳本 `Source` 從編輯過的資料夾**填回去**。
3. 重新序列化。

除了腳本原始碼以外的一切(幾何、非腳本物件、`Disabled` 旗標、其他所有屬性)都從那份
複本原封不動帶過來。

**因此你必須提供解壓時用的原始檔。** 這個工具是改程式碼,不是無中生有重建幾何。

## 用法

```bash
python pack.py <解壓資料夾> --original <原始.rbxl> [-o 輸出]
```

不指定 `-o` 時,輸出為 `<解壓資料夾>_packed<原始副檔名>`。輸出格式跟隨原始檔
(二進位進 → 二進位出,XML 進 → XML 出)。

## 腳本如何配對回去

解壓器輸出的 `manifest.json` 是**前序 DFS** 順序。`pack.py` 用同一套 rbx-dom crate
重新讀原始檔、以相同前序走訪、依序收集腳本 instance,再和 manifest 的腳本項目對齊。
配對是**依順序而非依名字**,所以即使同層物件同名也不會錯(例如兩個都叫 `Dup` 的資料夾
各有一個 `Twin` 腳本——這是依名字配對會出錯的情況)。每一對在覆寫 `Source` 前會再用
名字 + class 做完整性檢查;不符就中止而非寫壞。

## 需求

需要 Rust(`cargo`),首次執行自動 `cargo build --release`。忠實寫出二進位 `.rbxl`
需要 rbx-dom crates,所以(不像 `extract.py`)打包**沒有**純 Python fallback。

## 新模式(`--new`):從資料夾樹直接生成全新 place

全新專案(所有腳本都是新的、沒有原始檔可填)用 `--new` 直接從 `src/` 樹**建出全新** place:

```bash
python pack.py <專案目錄> --new [-o Game.rbxl]
```

`<專案目錄>/src` 存在時作為樹根,否則用 `<專案目錄>` 本身;不需要 `manifest.json`。
命名慣例與解壓輸出相同(`*.server.lua(u)` → `Script`、`*.client.lua(u)` →
`LocalScript`、`*.lua(u)` → `ModuleScript`、含 `init.*` 的目錄 → 帶子物件的腳本、
其他目錄 → `Folder`)。頂層目錄依名稱對應 Roblox 服務,並固定建立基線服務集讓
Studio 乾淨開啟。輸出格式跟隨輸出副檔名(`.rbxlx`/`.rbxmx` = XML,其他 = 二進位)。

Round-trip 測試:`python test/test_new_mode.py`。

## 範圍 / 限制

- **Fill 模式只處理腳本原始碼。** 在資料夾裡改名/新增/刪除物件不會反映;只會覆寫原始檔中
  既有腳本的 `Source`。(要新增腳本請在 Studio 加、重新解壓、再編輯——或全程式碼專案
  改用 `--new`。)
- Fill 模式需要**對應的**原始檔。用另一個版本的 place 解壓出的資料夾會過不了數量/完整性檢查。
- New 模式只產生腳本/Folder/服務——不含幾何與非預設屬性;其他東西一律由程式在執行期建立。
