# Fixture notes — Phase 0 reverse engineering

Analyzed 2026-07-24 against real user files.

## Source paths

| Role | Path |
| --- | --- |
| Clean Wonderprint template | `C:\Users\RyanB\Desktop\WonderClean.3mf` |
| Problem MakerWorld project | `C:\Users\RyanB\Documents\3D\Fidgets\toy+story+dumpling+box.3mf` |
| Extracted trees (local) | `fixtures/_extract/{clean,problem}/` |
| Lightweight metadata copies | `fixtures/samples/{clean,problem}/` |

**Do not commit** full problem geometry (≈30 MB object meshes, copyrighted MakerWorld model). Metadata samples only are fine for diffs.

---

## Package inventory

### Clean (`WonderClean.3mf`)

```text
[Content_Types].xml
_rels/.rels
3D/3dmodel.model                 # empty resources/build (no geometry)
Metadata/
  project_settings.config        # 31 KB JSON, 543 keys
  model_settings.config          # empty plate 1 only
  slice_info.config              # empty header
  plate_1.png, plate_1_small.png, plate_no_light_1.png, top_1.png, pick_1.png
```

- **Application:** `BambuStudio-2.3.1` (Wonderprint reports as BambuStudio lineage string)
- **Printer:** `WonderMaker ZR Ultra S` / variant `0.4`
- **Process:** `0.20mm Standard @WonderMaker ZR Ultra`
- **Filaments (4 independent toolheads):** PETG ×4, colours `#FFFFFF`, `#FFFF00`, `#FF0000`, `#0000FF`
- **Array arity:** almost everything is length **4** (`nozzle_diameter`, `extruder_colour`, filament arrays)
- **`single_extruder_multi_material`:** `"0"` (correct for independent toolheads)
- **Start G-code:** Klipper-style `START_PRINT EXTRUDER=... INITIAL_TOOL=...` (WonderMaker)
- **No** nested `3D/Objects/`, **no** Auxiliaries, **no** plate G-code

### Problem (`toy+story+dumpling+box.3mf`)

```text
[Content_Types].xml
_rels/.rels
3D/
  3dmodel.model                  # assembly of components
  Objects/object_21.model        # ~9.5 MB
  Objects/object_22.model        # ~1.9 MB
  Objects/object_35.model        # ~18.7 MB
  _rels/3dmodel.model.rels
Auxiliaries/                     # MakerWorld thumbnails, profile pics
Metadata/
  project_settings.config        # 74 KB JSON, 571 keys
  model_settings.config          # 45 KB — plates + objects + parts
  slice_info.config              # version header only (not fully sliced)
  filament_sequence.json
  cut_information.xml
  layer_heights_profile.txt
  plate_*.png, top_*.png, pick_*.png (plates 1–2)
```

- **Application:** `BambuStudio-02.07.01.62` (matches user warning)
- **Printer:** `Bambu Lab H2C` / variant `0.4` (matches screenshot)
- **Process:** `good top layer and support removal` (custom)
- **Filaments:** PLA ×4, **same colours** as clean: `#FFFFFF`, `#FFFF00`, `#FF0000`, `#0000FF`
- **Hardware model mismatch vs ZR:**
  - H2C: `nozzle_diameter` / `extruder_type` length **2** (dual nozzle) + AMS-style multi-filament
  - `single_extruder_multi_material`: `"1"`
  - `filament_map_mode`: `"Auto For Flush"`
  - Large H2C-specific `machine_start_gcode` / `change_filament_gcode` / AMS flush matrix
- **Warning-related keys (confirmed):**
  - `ensure_vertical_shell_thickness`: `"enabled"` (clean: `"ensure_all"`)
  - `support_style`: `"tree_organic"`
  - `raft_first_layer_expansion`: `"-1"` (clean: `"2"`)
- **195 keys only in problem**, **167 only in clean**, **184 shared keys differ** — whole-file `project_settings` replace is mandatory, not key patching.

---

## Color model (this fixture)

| Mechanism | Present? |
| --- | --- |
| Triangle `paint_color` | **No** (0 attrs in all object models) |
| Per-part `extruder` in `model_settings` | **Yes** |
| Separate colored mesh parts | **Yes** |

### Objects

| Object id | Name | Parts | Extruders used |
| --- | --- | --- | --- |
| 2 | Untitled.stl | 1 | 1 |
| 6 | Assembly_B_A | 3 | 1 |
| 66 | Keychain Draft.3mf | 59 | 1, 2, 3, 4 |

### Object 66 part counts by toolhead

| Extruder | Parts |
| --- | --- |
| 1 (white) | 51 |
| 2 (yellow) | 3 |
| 3 (red) | 3 |
| 4 (blue) | 2 |

**Default conversion map for this file: identity 1→1, 2→2, 3→3, 4→4.**  
Clean template already uses the same hex palette, so colour swatches will match out of the box; filament *type* (PLA→user PETG or keep PLA in template) is a UI choice when grafting.

### Plates

- **Plate 1:** 1 instance  
- **Plate 2:** 2 instances  

Multi-plate must be preserved via keeping `model_settings` + geometry.

---

## Implications for converter algorithm

### Confirmed: Strategy S1 (settings graft) fits this pair

1. **Keep** from problem:
   - Entire `3D/` tree (root model, Objects, rels)
   - `Metadata/model_settings.config` (plates, objects, parts, extruders, transforms)
   - Plate thumbnails (optional)
   - Optionally `Auxiliaries/` (cosmetic MakerWorld media; safe to keep)

2. **Replace** from clean template:
   - `Metadata/project_settings.config` **entire file**
   - Optionally patch template filament colours/types after replace if user remaps

3. **Strip / neutralize:**
   - `Metadata/slice_info.config` → empty Wonderprint-style header or rewrite Application version
   - `Metadata/filament_sequence.json` — H2C nozzle sequence; **drop or reset empty**
   - `Metadata/cut_information.xml` — keep if cut features needed; verify load (low risk)
   - `Metadata/layer_heights_profile.txt` — process-related; **prefer drop** so ZR process defaults apply cleanly
   - Any `*.gcode` (none in this fixture)

4. **Patch lightly:**
   - Root `3D/3dmodel.model` Application metadata → `BambuStudio-2.3.1` (match Wonderprint) *or leave* if loader ignores it after settings graft
   - On each plate in `model_settings`, set `filament_map_mode` / `filament_maps` consistent with ZR four toolheads (template plate uses `Auto For Flush` and `1 1 1 1` — may need `filament_maps` that match used slots; test open)
   - Remap `extruder` attributes only when user map ≠ identity

5. **Do not rewrite** large `.model` meshes for this fixture (no paint codes). Remap path still required for painted MakerWorld files later.

### H2C dual-nozzle vs ZR four-toolhead

| Concept | Problem (H2C) | Clean (ZR Ultra S) |
| --- | --- | --- |
| Physical nozzles / toolheads in settings | 2 | 4 |
| Filament slots | 4 | 4 |
| SEMM | 1 | 0 |
| Tool change G-code | H2C AMS/flush | empty / START_PRINT macros |
| Bed size | 330×320 | 300×270 |

Grafting clean `project_settings` fixes machine semantics. **Bed is smaller (300×270 vs 330×320)** — converter report should warn that objects near edges may sit outside ZR bed; user may need to rearrange.

---

## Acceptance test (manual, after first convert)

1. Convert problem + WonderClean → `toy+story+dumpling+box-zr-ultra-s.3mf`
2. Open in Wonderprint-Orca 2.3.1
3. Expect:
   - Printer: **WonderMaker ZR Ultra S 0.4**
   - No flood of `ensure_vertical_shell_thickness` / `raft_first_layer_expansion` / relative extruder warnings from H2C G-code
   - Four colours still assigned on Keychain object parts
   - Two plates present
4. Slice Preview on plate with multi-colour parts; confirm tool changes

---

## Open questions for first prototype

1. After graft, does Wonderprint honour part `extruder` values without also needing plate `filament_maps` tweaks?
2. Does keeping `layer_heights_profile.txt` re-apply incompatible adaptive heights?
3. Should filament *types* in output stay as template (PETG) or be overwritten from source PLA colours only?
4. Bed overflow: auto-detect AABB vs 300×270 and warn?

Recommended defaults: **template filament profiles/types**, **source colours** copied into template’s `filament_colour` / `filament_multi_colour` arrays for used slots, identity extruder map.
