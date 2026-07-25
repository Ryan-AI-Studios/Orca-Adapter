# Wondermaker 3MF Converter (Desktop)

Desktop shell for **Wondermaker 3MF Converter** (3MF Profile Transplant): a Tauri 2 + SvelteKit app that transplants MakerWorld / Bambu Studio project `.3mf` packages onto Wonderprint-Orca (ZR Ultra-S) printer settings while preserving geometry, plates, and multi-color assignments.

## What it does

- **S1 settings graft** (default): keep source geometry + `model_settings`, replace `project_settings` from your Wonderprint template, patch filament colours, remap toolheads / paint.
- **Local only** — conversion runs in-process via Tauri commands over `wondermaker_3mf_core`. No model upload, no cloud.
- **Never overwrites** the source project; output defaults to `{stem}-zr-ultra-s.3mf` beside the source.
- Strips embedded plate G-code and unsafe slice artifacts; you **re-slice** in Wonderprint-Orca before printing.

## Stack

- Tauri 2 (Windows / WebView2)
- SvelteKit + Vite frontend
- `wondermaker_3mf_core` for analyze + convert

## For end users (no server, no terminal)

The shipping app is a normal Windows program. **Nobody needs to start a dev server.**

From a built release (see below), give people one of:

| File | How to use |
| --- | --- |
| `dist\3MF Profile Transplant.exe` | Double-click — portable, no install |
| `dist\3MF Profile Transplant-setup.exe` | Run once, then open from Start menu |

Requirements: Windows 10/11 64-bit and [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) (usually already installed).

## Develop (hot reload — developers only)

```powershell
cd apps\desktop
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` starts Vite for live UI reload. That is **not** how end users run the app.

Frontend-only preview (no native convert):

```powershell
pnpm dev
```

## Build a double-clickable app

From the **repo root** (recommended — copies clean names into `dist\`):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-desktop-app.ps1
```

Or manually:

```powershell
cd apps\desktop
pnpm install
pnpm tauri build
```

Then open:

- Portable binary: `target\release\desktop.exe` (same as `dist\3MF Profile Transplant.exe` after the script)
- Installer: `target\release\bundle\nsis\*-setup.exe`
- MSI: `target\release\bundle\msi\*.msi`

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-analyzer.rust-analyzer).
