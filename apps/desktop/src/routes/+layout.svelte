<script lang="ts">
  import "../app.css";
  import { page } from "$app/stores";
  import { getCurrentWindow } from "@tauri-apps/api/window";

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
    { href: "/", label: "Convert", icon: "convert" },
    { href: "/help", label: "Help", icon: "help" },
  ];

  function isActive(href: string, pathname: string): boolean {
    if (href === "/") return pathname === "/" || pathname === "";
    return pathname.startsWith(href);
  }
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
    <aside class="sidebar" data-testid="sidebar">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true">
          <svg width="56" height="56" viewBox="0 0 56 56" fill="none">
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
            data-testid={item.icon === "convert" ? "nav-convert" : "nav-help"}
          >
            {#if item.icon === "convert"}
              <svg width="20" height="20" viewBox="0 0 20 20" fill="none" aria-hidden="true">
                <path d="M4 6h8M12 6l-2-2M12 6l-2 2M16 14H8M8 14l2-2M8 14l2 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
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

      <div class="sidebar-spacer"></div>

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
    padding: 16px 0 14px;
    border-right: 1px solid #102432;
  }

  .brand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 8px 14px 20px;
    min-height: 190px;
  }

  .brand-mark {
    width: 104px;
    height: 104px;
    border-radius: 18px;
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
    font-size: 18px;
    font-weight: 650;
    line-height: 25px;
    color: var(--text);
    text-align: center;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 14px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 60px;
    padding: 0 16px;
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

  .local-panel {
    margin: 0 20px 6px;
    height: 50px;
    border-radius: 8px;
    background: var(--green-surface);
    border: 1px solid var(--green-border);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 12px;
  }

  .local-dot {
    color: var(--green-local);
    font-size: 12px;
    line-height: 1;
  }

  .local-text {
    color: var(--green);
    font-size: 13px;
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
    .sidebar {
      width: var(--sidebar-collapsed);
      min-width: var(--sidebar-collapsed);
    }
    .brand-name,
    .nav-label,
    .local-text {
      display: none;
    }
    .brand {
      min-height: auto;
      padding-bottom: 12px;
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
    .local-panel {
      margin: 0 10px;
      width: auto;
    }
  }
</style>
