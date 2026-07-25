<script lang="ts">
  import "../app.css";
  import { page } from "$app/stores";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { homeShell } from "$lib/homeShell.svelte";
  import { formatBed, formatBytes } from "$lib/api";

  let { children } = $props();

  let maximized = $state(false);

  async function minimize() {
    try {
      await getCurrentWindow().minimize();
    } catch {
      /* non-tauri preview */
    }
  }

  async function toggleMaximize() {
    try {
      const win = getCurrentWindow();
      await win.toggleMaximize();
      maximized = await win.isMaximized();
    } catch {
      /* non-tauri preview */
    }
  }

  async function closeWin() {
    try {
      await getCurrentWindow().close();
    } catch {
      /* non-tauri preview */
    }
  }

  const nav = [
    { href: "/", label: "Home", icon: "home" },
    { href: "/help", label: "Help", icon: "help" },
  ];

  function isActive(href: string, pathname: string): boolean {
    if (href === "/") return pathname === "/" || pathname === "";
    return pathname.startsWith(href);
  }

  const onHome = $derived(isActive("/", $page.url.pathname));
  const showHomeChrome = $derived(onHome && homeShell.active);
</script>

<div class="shell">
  <header class="titlebar" data-testid="titlebar">
    <div class="titlebar-left">
      <div class="app-icon" aria-hidden="true">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none">
          <rect x="4" y="3" width="12" height="16" rx="1.5" stroke="currentColor" stroke-width="1.6" />
          <path d="M8 8h4M8 11h4M8 14h2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          <path d="M15 14l3-2.5L15 9M18 14l3-2.5L18 9" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </div>
      <span class="app-title">3MF Profile Transplant</span>
    </div>
    <div class="titlebar-drag" data-tauri-drag-region></div>
    <div class="window-controls">
      <button type="button" class="win-btn" aria-label="Minimize" onclick={minimize}>
        <svg width="12" height="12" viewBox="0 0 12 12"><path d="M2 6h8" stroke="currentColor" stroke-width="1.2" /></svg>
      </button>
      <button type="button" class="win-btn" aria-label="Maximize" onclick={toggleMaximize}>
        {#if maximized}
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M3.5 4.5h5v5h-5zM4 3.5h5.5v5.5" stroke="currentColor" stroke-width="1.1" />
          </svg>
        {:else}
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <rect x="2.5" y="2.5" width="7" height="7" stroke="currentColor" stroke-width="1.1" />
          </svg>
        {/if}
      </button>
      <button type="button" class="win-btn win-close" aria-label="Close" onclick={closeWin}>
        <svg width="12" height="12" viewBox="0 0 12 12">
          <path d="M3 3l6 6M9 3L3 9" stroke="currentColor" stroke-width="1.2" />
        </svg>
      </button>
    </div>
  </header>

  <div class="body">
    <aside class="sidebar" class:home-mode={showHomeChrome} data-testid="sidebar">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true">
          <svg width="48" height="48" viewBox="0 0 56 56" fill="none">
            <rect x="10" y="8" width="28" height="36" rx="3" stroke="var(--cyan)" stroke-width="2" />
            <path d="M18 18h12M18 24h12M18 30h8" stroke="var(--cyan)" stroke-width="1.8" stroke-linecap="round" opacity="0.85" />
            <path
              d="M36 30l8-6-8-6M42 30l8-6-8-6"
              stroke="var(--cyan)"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <text x="16" y="40" fill="var(--cyan)" font-size="8" font-weight="700" font-family="Segoe UI, sans-serif">3MF</text>
          </svg>
        </div>
        <div class="brand-name">
          <span>3MF Profile</span>
          <span>Transplant</span>
        </div>
      </div>

      <nav class="nav">
        {#each nav as item}
          <a
            href={item.href}
            class="nav-item"
            class:selected={isActive(item.href, $page.url.pathname)}
            data-testid={item.icon === "home" ? "nav-home" : "nav-help"}
          >
            {#if item.icon === "home"}
              <svg width="20" height="20" viewBox="0 0 20 20" fill="none" aria-hidden="true">
                <path
                  d="M3.5 9.5L10 3.5l6.5 6M5 8.5V16a1 1 0 001 1h3v-4h2v4h3a1 1 0 001-1V8.5"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            {:else}
              <svg width="20" height="20" viewBox="0 0 20 20" fill="none" aria-hidden="true">
                <circle cx="10" cy="10" r="7" stroke="currentColor" stroke-width="1.5" />
                <path d="M10 9v5M10 6.5v.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
              </svg>
            {/if}
            <span class="nav-label">{item.label}</span>
          </a>
        {/each}
      </nav>

      {#if showHomeChrome}
        <div class="side-analysis" data-testid="side-analysis">
          <div class="side-section-label">Analysis</div>

          {#if homeShell.analyzing}
            <div class="side-status">
              <span class="spinner sm"></span>
              <span>Analyzing…</span>
            </div>
          {:else if homeShell.analysisError}
            <div class="side-status err">{homeShell.analysisError}</div>
          {:else if homeShell.sourceAnalysis}
            <div class="side-chips">
              <span class="side-chip">{homeShell.sourceAnalysis.plateCount} plate{homeShell.sourceAnalysis.plateCount === 1 ? "" : "s"}</span>
              <span class="side-chip">{homeShell.sourceAnalysis.coloredParts} part{homeShell.sourceAnalysis.coloredParts === 1 ? "" : "s"}</span>
              <span class="side-chip">{homeShell.sourceAnalysis.colorCount} color{homeShell.sourceAnalysis.colorCount === 1 ? "" : "s"}</span>
              <span class="side-chip">{formatBytes(homeShell.sourceAnalysis.fileSizeBytes)}</span>
            </div>
          {:else}
            <p class="side-muted">Select a source project to analyze.</p>
          {/if}

          <table class="side-meta">
            <tbody>
              <tr>
                <th>Printer</th>
                <td>
                  {#if homeShell.sourceAnalysis || homeShell.templateAnalysis}
                    <span class="src">{homeShell.sourceAnalysis?.printerModel ?? "—"}</span>
                    <span class="arr">→</span>
                    <span class="dst">{homeShell.templateAnalysis?.printerModel ?? "—"}</span>
                  {:else}
                    —
                  {/if}
                </td>
              </tr>
              <tr>
                <th>Color mode</th>
                <td>{homeShell.sourceAnalysis?.colorMode ?? "—"}</td>
              </tr>
              <tr>
                <th>Source slicer</th>
                <td class="truncate" title={homeShell.sourceAnalysis?.application ?? ""}>
                  {homeShell.sourceAnalysis?.application ?? "—"}
                </td>
              </tr>
              <tr>
                <th>Target</th>
                <td class="dst">{homeShell.templateAnalysis?.printerModel ?? "—"}</td>
              </tr>
              <tr>
                <th>Bed</th>
                <td>
                  {formatBed(homeShell.sourceAnalysis?.bedSizeMm)}
                  {#if homeShell.templateAnalysis?.bedSizeMm}
                    <span class="arr">→</span>
                    <span class="dst">{formatBed(homeShell.templateAnalysis.bedSizeMm)}</span>
                  {/if}
                </td>
              </tr>
            </tbody>
          </table>

          {#if homeShell.sourceAnalysis && homeShell.sourceAnalysis.plateCount > 0}
            <div class="side-plates">
              {#each Array(homeShell.sourceAnalysis.plateCount) as _, i}
                {@const n = i + 1}
                {@const thumb = homeShell.plateThumbs[n]}
                {#if thumb}
                  <button
                    type="button"
                    class="side-plate has-thumb"
                    onclick={() => homeShell.onOpenPlate?.(n)}
                    title="Enlarge plate {n}"
                    aria-label="Enlarge plate {n} preview"
                  >
                    <img src={thumb.dataUrl} alt="Plate {n}" />
                    <span>P{n}</span>
                  </button>
                {:else}
                  <div class="side-plate">
                    <span>P{n}</span>
                    <span class="no-prev">No preview</span>
                  </div>
                {/if}
              {/each}
            </div>
          {/if}

          {#if homeShell.bedWarning}
            <div class="side-bed-warn">{homeShell.bedWarning}</div>
          {/if}

          {#if homeShell.sourceAnalysis?.warnings?.length}
            <ul class="side-warnings">
              {#each homeShell.sourceAnalysis.warnings as w}
                <li>{w}</li>
              {/each}
            </ul>
          {/if}
        </div>
      {:else}
        <div class="sidebar-spacer"></div>
      {/if}

      {#if showHomeChrome}
        <div class="side-convert-wrap">
          <button
            type="button"
            class="btn-side-convert"
            data-testid="convert-btn"
            disabled={!homeShell.canConvert}
            onclick={() => homeShell.onConvert?.()}
          >
            {#if homeShell.converting}
              <span class="spinner sm"></span>
              {homeShell.progressStage ? homeShell.progressStage : "Converting…"}
            {:else}
              Convert project
            {/if}
          </button>
        </div>
      {/if}

      <div class="local-panel" data-testid="local-only" title="Conversion runs entirely on this PC">
        <span class="local-dot">●</span>
        <span class="local-text">Local only • No uploads</span>
      </div>
    </aside>

    <main class="main main-scroll">
      {@render children()}
    </main>
  </div>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
    background: var(--canvas);
  }

  .titlebar {
    height: var(--titlebar-h);
    min-height: var(--titlebar-h);
    background: var(--titlebar);
    border-bottom: 1px solid #102432;
    display: flex;
    align-items: center;
    padding: 0 0 0 12px;
    z-index: 50;
  }

  .titlebar-left {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 8px;
    flex-shrink: 0;
  }

  .app-icon {
    width: 22px;
    height: 22px;
    color: var(--cyan);
    display: grid;
    place-items: center;
  }

  .app-title {
    font-size: 15px;
    font-weight: 500;
    color: var(--text);
    letter-spacing: 0.01em;
  }

  .titlebar-drag {
    flex: 1;
    height: 100%;
    min-width: 40px;
  }

  .window-controls {
    display: flex;
    height: 100%;
    flex-shrink: 0;
  }

  .win-btn {
    width: 46px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    display: grid;
    place-items: center;
    padding: 0;
  }

  .win-btn:hover {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
  }

  .win-close:hover {
    background: #e81123;
    color: #fff;
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .sidebar {
    width: var(--sidebar-w);
    min-width: var(--sidebar-w);
    background: var(--sidebar);
    display: flex;
    flex-direction: column;
    padding: 12px 0 12px;
    border-right: 1px solid #102432;
    min-height: 0;
  }

  .sidebar.home-mode {
    width: var(--sidebar-home-w);
    min-width: var(--sidebar-home-w);
  }

  .brand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 4px 14px 12px;
    flex-shrink: 0;
  }

  .brand-mark {
    width: 72px;
    height: 72px;
    border-radius: 14px;
    background: var(--card);
    border: 1.5px solid var(--cyan);
    display: grid;
    place-items: center;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
  }

  .brand-name {
    display: flex;
    flex-direction: column;
    align-items: center;
    font-size: 15px;
    font-weight: 650;
    line-height: 20px;
    color: var(--text);
    text-align: center;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 12px;
    flex-shrink: 0;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 48px;
    padding: 0 14px;
    border-radius: 8px;
    color: var(--text-secondary);
    text-decoration: none;
    font-size: 15px;
    font-weight: 500;
    border-left: 3px solid transparent;
    transition: background 0.12s ease;
  }

  .nav-item:hover {
    background: var(--nav-hover);
    color: var(--text);
  }

  .nav-item.selected {
    background: var(--nav-selected);
    color: var(--text);
    border-left-color: var(--cyan);
  }

  .sidebar-spacer {
    flex: 1;
  }

  .side-analysis {
    flex: 1;
    min-height: 0;
    overflow: auto;
    margin: 10px 10px 0;
    padding: 12px 12px 10px;
    border-radius: 8px;
    background: var(--surface-1);
    border: 1px solid #163040;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .side-section-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-label);
  }

  .side-status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .side-status.err {
    color: var(--red);
    line-height: 1.35;
  }

  .side-muted {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
    line-height: 1.35;
  }

  .side-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .side-chip {
    font-size: 11px;
    padding: 4px 8px;
    border-radius: 5px;
    background: var(--chip-bg);
    border: 1px solid var(--chip-border);
    color: var(--text-secondary);
  }

  .side-meta {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .side-meta th {
    text-align: left;
    width: 34%;
    padding: 5px 6px 5px 0;
    color: var(--text-muted);
    font-weight: 500;
    vertical-align: top;
    border-bottom: 1px solid #1a2e3a;
  }

  .side-meta td {
    padding: 5px 0;
    color: var(--text-secondary);
    border-bottom: 1px solid #1a2e3a;
    word-break: break-word;
  }

  .side-meta .src {
    color: var(--text-secondary);
  }

  .side-meta .dst {
    color: var(--cyan);
  }

  .side-meta .arr {
    margin: 0 3px;
    color: var(--text-muted);
  }

  .side-meta .truncate {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .side-plates {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .side-plate {
    width: 64px;
    height: 52px;
    border-radius: 5px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    position: relative;
    padding: 0;
  }

  .side-plate.has-thumb {
    cursor: zoom-in;
    color: #fff;
  }

  .side-plate.has-thumb:hover {
    border-color: var(--cyan);
  }

  .side-plate img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .side-plate.has-thumb span {
    position: relative;
    z-index: 1;
    background: rgba(0, 0, 0, 0.55);
    padding: 1px 5px;
    border-radius: 3px;
    font-weight: 600;
  }

  .side-plate .no-prev {
    font-size: 9px;
  }

  .side-bed-warn {
    font-size: 11px;
    line-height: 1.35;
    color: var(--amber-text);
    background: var(--amber-surface);
    border: 1px solid var(--amber-border);
    border-radius: 5px;
    padding: 7px 8px;
  }

  .side-warnings {
    margin: 0;
    padding-left: 16px;
    font-size: 11px;
    color: var(--amber-text);
    line-height: 1.35;
  }

  .side-convert-wrap {
    padding: 10px 12px 6px;
    flex-shrink: 0;
  }

  .btn-side-convert {
    width: 100%;
    height: 48px;
    border: 1px solid var(--cyan-btn-border);
    border-radius: 8px;
    background: var(--cyan-btn);
    color: #04121d;
    font-size: 15px;
    font-weight: 700;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    cursor: pointer;
    padding: 0 12px;
  }

  .btn-side-convert:hover:not(:disabled) {
    background: var(--cyan-btn-hover);
  }

  .btn-side-convert:active:not(:disabled) {
    background: var(--cyan-btn-pressed);
  }

  .btn-side-convert:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid rgba(4, 18, 29, 0.25);
    border-top-color: #04121d;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  .spinner.sm {
    width: 14px;
    height: 14px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .local-panel {
    margin: 4px 12px 4px;
    min-height: 42px;
    border-radius: 8px;
    background: var(--green-surface);
    border: 1px solid var(--green-border);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 10px;
    flex-shrink: 0;
  }

  .local-dot {
    color: var(--green-local);
    font-size: 12px;
    line-height: 1;
  }

  .local-text {
    color: var(--green);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }

  .main {
    flex: 1;
    min-width: 0;
    overflow: auto;
    background:
      radial-gradient(1200px 600px at 20% -10%, rgba(42, 214, 223, 0.04), transparent 55%),
      var(--canvas);
    padding: 18px 22px 28px;
  }

  @media (max-width: 1180px) {
    .sidebar,
    .sidebar.home-mode {
      width: var(--sidebar-collapsed);
      min-width: var(--sidebar-collapsed);
    }
    .brand-name,
    .nav-label,
    .local-text,
    .side-analysis,
    .side-convert-wrap .btn-side-convert {
      /* keep convert icon-ish: still show button label truncated */
    }
    .brand-name,
    .nav-label,
    .local-text {
      display: none;
    }
    .side-analysis {
      display: none;
    }
    .brand {
      min-height: auto;
      padding-bottom: 8px;
    }
    .brand-mark {
      width: 44px;
      height: 44px;
      border-radius: 10px;
    }
    .brand-mark svg {
      width: 28px;
      height: 28px;
    }
    .nav-item {
      justify-content: center;
      padding: 0;
      border-left-width: 0;
      border-bottom: 2px solid transparent;
    }
    .nav-item.selected {
      border-bottom-color: var(--cyan);
      border-left-color: transparent;
    }
    .btn-side-convert {
      font-size: 11px;
      height: 40px;
      padding: 0 4px;
    }
    .local-panel {
      margin: 0 8px;
      width: auto;
    }
  }
</style>
