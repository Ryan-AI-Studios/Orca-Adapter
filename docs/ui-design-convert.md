# 3MF Profile Transplant — Convert page design (reference)

Canonical implementation binding lives in `conductor/0004-Tauri2DesktopUi/`. This file is a short index of the full mockup-driven UI contract for implementers.

## Window

- Target ~1440×1020; min 1100×800  
- Title bar 44 px · Sidebar 256 px (Home: 300 px with analysis) · Main remainder  
- Spacing: 20 px large / 8 px internal  

## Convert vertical order

1. Heading (~82 px)  
2. Source | Template cards (~180 px, 1:1 grid, gap 10)  
3. Compact analysis-status strip  
4. Toolhead mapping (full width)  
5. Safety summary  
6. Output options + path (Convert button lives in the **left sidebar** under Help)  

**Left sidebar (Home):** nav (Home · Help) → project analysis (printer, bed, plates, chips) → **Convert project** → Local only badge.

## Hard rules

- Metrics, badges, warnings, and G-code safety copy are **data-driven** from Rust core.  
- No invented plate thumbnails.  
- No “ready to print” — re-slice only.  
- No TypeScript 3MF conversion.  
- History optional/disabled; not MVP.  

See `conductor/0004-Tauri2DesktopUi/plan.md` §§2–8 for tokens, states, responsive, and command DTOs.
