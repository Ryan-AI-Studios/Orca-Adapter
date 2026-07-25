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

## Develop

```powershell
cd apps\desktop
pnpm install
pnpm tauri dev
```

Frontend-only preview (no native convert):

```powershell
pnpm dev
```

## Build

```powershell
cd apps\desktop
pnpm build
pnpm tauri build
```

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-analyzer.rust-analyzer).
