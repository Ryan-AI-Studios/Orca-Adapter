# 3MF Profile Transplant — Convert page design (reference)

Canonical implementation binding lives in `conductor/0004-Tauri2DesktopUi/`. This file is a short index of the full mockup-driven UI contract for implementers.

## Window

- Target ~1440×900; min 1100×720  
- Title bar 44 px · Sidebar 256 px · Main remainder  
- Spacing: 20 px large / 8 px internal  

## Convert vertical order

1. Heading (~82 px)  
2. Source | Template cards (~180 px, 1:1 grid, gap 10)  
3. Analysis-status strip (50 px, gap 10 above)  
4. Project analysis | Toolhead mapping (~385 px, gap 10)  
5. Safety summary (~62 px)  
6. Output / Convert row (~106 px)  

## Hard rules

- Metrics, badges, warnings, and G-code safety copy are **data-driven** from Rust core.  
- No invented plate thumbnails.  
- No “ready to print” — re-slice only.  
- No TypeScript 3MF conversion.  
- History optional/disabled; not MVP.  

See `conductor/0004-Tauri2DesktopUi/plan.md` §§2–8 for tokens, states, responsive, and command DTOs.
