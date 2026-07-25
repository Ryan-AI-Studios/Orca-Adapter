<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    analyze3mf,
    basename,
    buildSlotMapString,
    convert3mf,
    extractPlateThumbnails,
    formatBed,
    getConfig,
    normalizeHex,
    openOutputFolder,
    pathExists,
    setTemplatePath,
    suggestOutputPath,
    usedSourceSlots,
    validateTemplate,
  } from "$lib/api";
  import { clearHomeShell, publishHomeShell } from "$lib/homeShell.svelte";
  import type {
    AnalysisDto,
    ConversionReportDto,
    FlowPhase,
    PlateThumbnailDto,
    ProgressEvent,
  } from "$lib/types";

  type DropTarget = "source" | "template" | null;

  let sourcePath = $state<string | null>(null);
  let templatePath = $state<string | null>(null);
  let sourceAnalysis = $state<AnalysisDto | null>(null);
  let templateAnalysis = $state<AnalysisDto | null>(null);
  let analysisError = $state<string | null>(null);
  let templateError = $state<string | null>(null);
  let convertError = $state<string | null>(null);
  let convertErrorDetail = $state<string | null>(null);
  let analyzing = $state(false);
  let converting = $state(false);
  let report = $state<ConversionReportDto | null>(null);
  let outputPath = $state("");
  let progressStage = $state<string | null>(null);
  let showDetails = $state(false);
  let showErrorDetail = $state(false);
  let dragOver = $state<DropTarget>(null);
  let slotDest = $state<Record<number, number>>({});
  let sourceCardEl = $state<HTMLElement | null>(null);
  let templateCardEl = $state<HTMLElement | null>(null);
  let copyPathLabel = $state("Copy path");
  /** plateIndex → thumbnail DTO */
  let plateThumbs = $state<Record<number, PlateThumbnailDto>>({});
  /** Which mapping row has its toolhead menu open (source slot), or null. */
  let openThMenu = $state<number | null>(null);
  /** Enlarged plate preview (lightbox). */
  let plateLightbox = $state<PlateThumbnailDto | null>(null);
  /** Opt-in markdown conversion report (default off). */
  let writeReport = $state(false);
  /**
   * When false (default): keep template filament colours on TH1–4 (matches UI swatches).
   * When true: copy MakerWorld source colours onto toolheads after mapping.
   */
  let copySourceColours = $state(false);

  function closeThMenus() {
    openThMenu = null;
  }

  function selectToolhead(slot: number, th: number) {
    if (th >= 1 && th <= 4) {
      slotDest = { ...slotDest, [slot]: th };
    }
    openThMenu = null;
  }

  function openPlatePreview(n: number) {
    const t = plateThumbs[n];
    if (t) plateLightbox = t;
  }

  const phase = $derived.by((): FlowPhase => {
    if (converting) return "converting";
    if (report) return "success";
    if (convertError) return "error";
    if (analyzing) return "analyzing";
    if (sourceAnalysis && templateAnalysis && !analysisError) return "ready";
    if (templatePath) return "template";
    if (sourcePath) return "source";
    return "empty";
  });

  const usedSlots = $derived(sourceAnalysis ? usedSourceSlots(sourceAnalysis) : []);

  const mapComplete = $derived(
    usedSlots.length > 0 && usedSlots.every((s) => slotDest[s] != null && slotDest[s] >= 1 && slotDest[s] <= 4),
  );

  const mergeDests = $derived.by(() => {
    const counts: Record<number, number[]> = {};
    for (const s of usedSlots) {
      const d = slotDest[s] ?? s;
      if (!counts[d]) counts[d] = [];
      counts[d].push(s);
    }
    return Object.entries(counts)
      .filter(([, srcs]) => srcs.length > 1)
      .map(([d]) => Number(d));
  });

  const bedWarning = $derived.by(() => {
    const sb = sourceAnalysis?.bedSizeMm;
    const tb = templateAnalysis?.bedSizeMm;
    if (!sb || !tb) return null;
    if (sb[0] > tb[0] + 0.5 || sb[1] > tb[1] + 0.5) {
      return `Build area differs: source ${Math.round(sb[0])} × ${Math.round(sb[1])} mm; target ${Math.round(tb[0])} × ${Math.round(tb[1])} mm. Verify placement before slicing.`;
    }
    return null;
  });

  function is3mfPath(p: string): boolean {
    return p.trim().toLowerCase().endsWith(".3mf");
  }

  const canConvert = $derived(
    !!sourcePath &&
      !!templatePath &&
      !!sourceAnalysis &&
      !!templateAnalysis &&
      !analyzing &&
      !converting &&
      !analysisError &&
      mapComplete &&
      !!outputPath.trim() &&
      is3mfPath(outputPath) &&
      outputPath.trim().toLowerCase() !== sourcePath.toLowerCase() &&
      outputPath.trim().toLowerCase() !== templatePath.toLowerCase(),
  );

  function filamentForSlot(slot: number): { colour: string; type: string } {
    const f = sourceAnalysis?.filaments.find((x) => x.index1based === slot);
    return { colour: f?.colour ?? "#888888", type: f?.type ?? "" };
  }

  function initMapFromAnalysis(a: AnalysisDto) {
    const slots = usedSourceSlots(a);
    const next: Record<number, number> = {};
    for (const s of slots) {
      // Default: identity when ≤4; extras map to TH4 (explicit, not silent drop)
      next[s] = s <= 4 ? s : 4;
    }
    slotDest = next;
  }

  async function pickSource() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "3MF project", extensions: ["3mf"] }],
        title: "Choose source project",
      });
      if (typeof selected === "string") await setSource(selected);
    } catch (e) {
      analysisError = String(e);
    }
  }

  async function pickTemplate() {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: "3MF template", extensions: ["3mf"] }],
        title: "Choose Wonderprint template",
      });
      if (typeof selected === "string") await setTemplate(selected);
    } catch (e) {
      templateError = String(e);
    }
  }

  async function pickOutputDir() {
    try {
      const dir = await open({
        directory: true,
        multiple: false,
        title: "Choose output folder",
      });
      if (typeof dir === "string" && sourcePath) {
        const stem = basename(sourcePath).replace(/\.3mf$/i, "");
        const sep = dir.includes("\\") ? "\\" : "/";
        outputPath = `${dir.replace(/[\\/]$/, "")}${sep}${stem}-zr-ultra-s.3mf`;
      }
    } catch {
      /* ignore cancel */
    }
  }

  async function setSource(path: string) {
    sourcePath = path;
    sourceAnalysis = null;
    plateThumbs = {};
    report = null;
    convertError = null;
    analysisError = null;
    analyzing = true;
    try {
      const a = await analyze3mf(path);
      sourceAnalysis = a;
      initMapFromAnalysis(a);
      outputPath = await suggestOutputPath(path);
      // Load plate previews after analysis (non-blocking for mapping UX)
      try {
        const thumbs = await extractPlateThumbnails(path, a.plateCount || 0);
        const map: Record<number, PlateThumbnailDto> = {};
        for (const t of thumbs) map[t.plateIndex] = t;
        plateThumbs = map;
      } catch {
        plateThumbs = {};
      }
    } catch (e) {
      analysisError = String(e);
      sourceAnalysis = null;
      plateThumbs = {};
    } finally {
      analyzing = false;
    }
  }

  /** Template filament for toolhead 1..=4 (from Wonderprint project settings). */
  function templateFilament(th: number): { colour: string; type: string } | null {
    if (!templateAnalysis?.filaments?.length) return null;
    const f = templateAnalysis.filaments.find((x) => x.index1based === th);
    if (f) return { colour: f.colour, type: f.type };
    // Fall back to 0-based array order
    const byIdx = templateAnalysis.filaments[th - 1];
    return byIdx ? { colour: byIdx.colour, type: byIdx.type } : null;
  }

  async function setTemplate(path: string) {
    templatePath = path;
    templateAnalysis = null;
    templateError = null;
    report = null;
    convertError = null;
    try {
      const a = await validateTemplate(path);
      templateAnalysis = a;
      await setTemplatePath(path);
    } catch (e) {
      templateError = String(e);
      templateAnalysis = null;
    }
  }

  async function runConvert() {
    if (!canConvert || !sourcePath || !templatePath) return;
    const out = outputPath.trim();
    try {
      const exists = await pathExists(out);
      if (exists) {
        const ok = window.confirm("Output already exists. Overwrite?");
        if (!ok) return;
      }
    } catch {
      // If existence check fails, proceed; convert will surface real errors.
    }

    converting = true;
    convertError = null;
    convertErrorDetail = null;
    report = null;
    progressStage = null;
    try {
      const r = await convert3mf({
        source: sourcePath,
        template: templatePath,
        output: out,
        slotMap: buildSlotMapString(slotDest, usedSlots),
        copySourceColours,
        copyFilamentType: true,
        writeReport,
        strictBed: false,
        strategy: "auto",
      });
      report = r;
      progressStage = null;
      copyPathLabel = "Copy path";
    } catch (e) {
      convertError = "Conversion failed";
      convertErrorDetail = String(e);
      progressStage = null;
    } finally {
      converting = false;
    }
  }

  function convertAnother() {
    report = null;
    convertError = null;
    convertErrorDetail = null;
    progressStage = null;
  }

  async function openFolder() {
    const p = report?.output || outputPath;
    if (p) await openOutputFolder(p);
  }

  async function openReport() {
    if (report?.reportPath) await openOutputFolder(report.reportPath);
  }

  async function copyOutputPath() {
    const p = report?.output;
    if (!p) return;
    try {
      await navigator.clipboard.writeText(p);
      copyPathLabel = "Copied";
      setTimeout(() => {
        copyPathLabel = "Copy path";
      }, 1500);
    } catch {
      copyPathLabel = "Copy failed";
      setTimeout(() => {
        copyPathLabel = "Copy path";
      }, 1500);
    }
  }

  /** Hit-test physical drop position against source/template card rects. */
  function resolveDropTarget(position: { x: number; y: number } | undefined): DropTarget {
    if (position && sourceCardEl && templateCardEl) {
      const dpr = window.devicePixelRatio || 1;
      const x = position.x / dpr;
      const y = position.y / dpr;
      const src = sourceCardEl.getBoundingClientRect();
      const tpl = templateCardEl.getBoundingClientRect();
      const inRect = (r: DOMRect) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      if (inRect(src)) return "source";
      if (inRect(tpl)) return "template";
    }
    // Fallback: empty source first, else template
    if (!sourcePath) return "source";
    return "template";
  }

  function assignDrop(path: string, target: DropTarget) {
    const lower = path.toLowerCase();
    if (!lower.endsWith(".3mf")) {
      analysisError = "Only .3mf files are supported";
      return;
    }
    if (target === "template") {
      void setTemplate(path);
    } else {
      void setSource(path);
    }
  }

  // Publish analysis + convert action into the left shell while Home is open.
  $effect(() => {
    publishHomeShell({
      active: true,
      canConvert,
      converting,
      analyzing,
      progressStage,
      analysisError,
      bedWarning,
      sourceAnalysis,
      templateAnalysis,
      plateThumbs,
      onConvert: () => {
        void runConvert();
      },
      onOpenPlate: (n) => openPlatePreview(n),
    });
  });

  onMount(() => {
    const unsubs: UnlistenFn[] = [];
    let disposed = false;

    (async () => {
      try {
        const cfg = await getConfig();
        if (cfg.templatePath && !disposed) {
          await setTemplate(cfg.templatePath);
        }
      } catch {
        /* first run */
      }

      try {
        const u1 = await listen<ProgressEvent>("convert-progress", (ev) => {
          progressStage = ev.payload.stage;
        });
        unsubs.push(u1);
      } catch {
        /* events optional in browser preview */
      }

      // Tauri 2 file drop on webview — hit-test by cursor position (IR1-03)
      try {
        const webview = getCurrentWebview();
        const u2 = await webview.onDragDropEvent((event) => {
          const payload = event.payload;
          const t = payload.type;
          if (t === "enter" || t === "over") {
            dragOver = resolveDropTarget(payload.position);
          } else if (t === "leave") {
            dragOver = null;
          } else if (t === "drop") {
            const target = resolveDropTarget(payload.position);
            dragOver = null;
            const paths = payload.paths ?? [];
            const first = paths.find((p) => p.toLowerCase().endsWith(".3mf")) ?? paths[0];
            if (first) assignDrop(first, target);
          }
        });
        unsubs.push(u2);
      } catch {
        /* non-tauri */
      }
    })();

    return () => {
      disposed = true;
      for (const u of unsubs) u();
      clearHomeShell();
    };
  });

  function sourceBadge(a: AnalysisDto | null): string {
    if (!a) return "";
    if (a.application) {
      if (/bambu/i.test(a.application)) return "Bambu Studio";
      if (/orca/i.test(a.application)) return "Orca";
      return a.application.length > 28 ? a.application.slice(0, 28) + "…" : a.application;
    }
    return a.printerModel ?? "Project";
  }

  function templateBadge(a: AnalysisDto | null): string {
    if (!a) return "";
    const printer = a.printerModel ?? "Wonderprint";
    return `${printer} • template`;
  }
</script>

<div class="convert-page">
  <header class="page-heading">
    <h1>Convert Bambu 3MF</h1>
    <p class="subtitle">
      Preserve models, plates, and color assignments. Replace incompatible printer settings.
    </p>
  </header>

  <!-- Source | Template -->
  <section class="file-cards">
    <div
      class="file-card"
      class:drag-over={dragOver === "source"}
      role="region"
      aria-label="Source project"
      data-testid="source-card"
      bind:this={sourceCardEl}
    >
      <div class="card-label">SOURCE PROJECT</div>
      {#if sourcePath && sourceAnalysis}
        <div class="card-loaded">
          <div class="doc-icon" aria-hidden="true">
            <svg width="40" height="48" viewBox="0 0 40 48" fill="none">
              <path d="M4 2h22l10 10v34a2 2 0 01-2 2H4a2 2 0 01-2-2V4a2 2 0 012-2z" stroke="var(--cyan)" stroke-width="1.6" fill="var(--surface-1)" />
              <path d="M26 2v10h10" stroke="var(--cyan)" stroke-width="1.6" />
            </svg>
          </div>
          <div class="card-meta">
            <div class="card-filename">{sourceAnalysis.fileName}</div>
            <div class="card-path" title={sourcePath}>{sourcePath}</div>
            <span class="badge cyan">{sourceBadge(sourceAnalysis)}</span>
          </div>
          <button type="button" class="btn-secondary" data-testid="source-browse" onclick={pickSource} disabled={converting}>Browse</button>
        </div>
      {:else}
        <div class="card-empty">
          <div class="empty-icon" aria-hidden="true">
            <svg width="36" height="36" viewBox="0 0 36 36" fill="none">
              <path d="M8 6h14l6 6v18a2 2 0 01-2 2H8a2 2 0 01-2-2V8a2 2 0 012-2z" stroke="var(--text-muted)" stroke-width="1.5" />
              <path d="M18 14v10M13 19h10" stroke="var(--cyan)" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </div>
          <div class="empty-title">Choose source project</div>
          <button type="button" class="btn-secondary" data-testid="source-browse" onclick={pickSource} disabled={converting}>Browse</button>
        </div>
      {/if}
      <div class="drop-strip">Drag &amp; drop a Bambu Studio .3mf file here</div>
    </div>

    <div
      class="file-card"
      class:drag-over={dragOver === "template"}
      role="region"
      aria-label="Wonderprint template"
      data-testid="template-card"
      bind:this={templateCardEl}
    >
      <div class="card-label">WONDERPRINT TEMPLATE</div>
      {#if templatePath && templateAnalysis}
        <div class="card-loaded">
          <div class="doc-icon" aria-hidden="true">
            <svg width="40" height="48" viewBox="0 0 40 48" fill="none">
              <path d="M4 2h22l10 10v34a2 2 0 01-2 2H4a2 2 0 01-2-2V4a2 2 0 012-2z" stroke="var(--green)" stroke-width="1.6" fill="var(--surface-1)" />
              <path d="M26 2v10h10" stroke="var(--green)" stroke-width="1.6" />
            </svg>
          </div>
          <div class="card-meta">
            <div class="card-filename">{templateAnalysis.fileName}</div>
            <div class="card-path" title={templatePath}>{templatePath}</div>
            <span class="badge green">{templateBadge(templateAnalysis)}</span>
          </div>
          <button type="button" class="btn-secondary" data-testid="template-browse" onclick={pickTemplate} disabled={converting}>Browse</button>
        </div>
      {:else}
        <div class="card-empty">
          <div class="empty-icon" aria-hidden="true">
            <svg width="36" height="36" viewBox="0 0 36 36" fill="none">
              <path d="M8 6h14l6 6v18a2 2 0 01-2 2H8a2 2 0 01-2-2V8a2 2 0 012-2z" stroke="var(--text-muted)" stroke-width="1.5" />
              <path d="M18 14v10M13 19h10" stroke="var(--green)" stroke-width="1.6" stroke-linecap="round" />
            </svg>
          </div>
          <div class="empty-title">Choose Wonderprint template</div>
          <button type="button" class="btn-secondary" data-testid="template-browse" onclick={pickTemplate} disabled={converting}>Browse</button>
        </div>
      {/if}
      {#if templateError}
        <div class="inline-error">{templateError}</div>
      {/if}
      <div class="drop-strip">Drag &amp; drop a Wonderprint-Orca .3mf template here</div>
    </div>
  </section>

  <!-- Compact status (full analysis lives in the left sidebar) -->
  <section
    class="status-strip"
    class:error={!!analysisError}
    class:ok={!!sourceAnalysis && !analyzing && !analysisError}
    data-testid="status-strip"
  >
    {#if analyzing}
      <div class="status-row">
        <div class="spinner"></div>
        <span class="status-msg analyzing">Analyzing project…</span>
      </div>
    {:else if analysisError}
      <div class="status-row">
        <span class="status-icon err">!</span>
        <span class="status-msg err">{analysisError}</span>
        <button type="button" class="link-btn" onclick={() => (showErrorDetail = !showErrorDetail)}>
          {showErrorDetail ? "Hide" : "View details"}
        </button>
      </div>
      {#if showErrorDetail}
        <pre class="error-detail">{analysisError}</pre>
      {/if}
    {:else if sourceAnalysis}
      <div class="status-row">
        <span class="status-icon ok" aria-hidden="true">✓</span>
        <span class="status-msg ok">Analysis complete — see left panel for project details</span>
      </div>
    {:else}
      <div class="status-row">
        <span class="status-msg muted">Select a source project to analyze</span>
      </div>
    {/if}
  </section>

  <!-- Toolhead mapping (analysis moved to sidebar) -->
  <section class="mid-grid single">
    <div class="panel" data-testid="map-panel">
      <h2 class="panel-heading">Toolhead mapping</h2>
      {#if !sourceAnalysis}
        <p class="muted-help">Analyze a source project to map colors to ZR Ultra-S toolheads (TH1–4).</p>
      {:else}
        {#if templateAnalysis}
          <div class="th-legend" aria-label="Template toolhead filament colors">
            <span class="th-legend-label">Your toolheads (template)</span>
            {#each [1, 2, 3, 4] as th}
              {@const tf = templateFilament(th)}
              <div class="th-legend-item" title={tf ? `${tf.type || "filament"} ${normalizeHex(tf.colour)}` : "TH" + th}>
                <span class="swatch sm" style:background={normalizeHex(tf?.colour ?? "")}></span>
                <span class="th-legend-th">TH{th}</span>
              </div>
            {/each}
          </div>
          <p class="muted-help sm">
            Left = model colours. Right = toolhead filaments from your template (kept in the output unless you enable “Copy source colours”).
          </p>
        {:else}
          <p class="muted-help sm">Select a Wonderprint template to show toolhead filament colors.</p>
        {/if}
        <!-- Close open TH menus when clicking outside -->
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          class="map-rows"
          class:disabled={analyzing || converting}
          onclick={(e) => {
            if (!(e.target as HTMLElement).closest?.(".th-picker")) closeThMenus();
          }}
        >
          {#each usedSlots as slot}
            {@const fil = filamentForSlot(slot)}
            {@const destTh = slotDest[slot] ?? (slot <= 4 ? slot : 4)}
            {@const tplFil = templateFilament(destTh)}
            <div class="map-row">
              <div class="src-box">
                <span class="slot-label">Slot {slot}</span>
                <span class="swatch" style:background={normalizeHex(fil.colour)} title={normalizeHex(fil.colour)}></span>
                {#if fil.type}
                  <span class="ftype">{fil.type}</span>
                {/if}
              </div>
              <span class="map-arrow" aria-hidden="true">→</span>
              <div class="th-box">
                <div class="th-picker" class:open={openThMenu === slot}>
                  <button
                    type="button"
                    class="th-picker-btn"
                    disabled={analyzing || converting}
                    aria-haspopup="listbox"
                    aria-expanded={openThMenu === slot}
                    aria-label="Map source slot {slot} to toolhead {destTh}"
                    onclick={(e) => {
                      e.stopPropagation();
                      openThMenu = openThMenu === slot ? null : slot;
                    }}
                  >
                    <span
                      class="swatch"
                      style:background={normalizeHex(tplFil?.colour ?? "")}
                      title={tplFil ? `Template TH${destTh}: ${normalizeHex(tplFil.colour)}` : `TH${destTh}`}
                    ></span>
                    <span class="th-picker-label">TH{destTh}</span>
                    {#if tplFil?.type}
                      <span class="ftype">{tplFil.type}</span>
                    {/if}
                    <span class="th-caret" aria-hidden="true">▾</span>
                  </button>
                  {#if openThMenu === slot}
                    <ul class="th-menu" role="listbox" aria-label="Toolheads">
                      {#each [1, 2, 3, 4] as th}
                        {@const optFil = templateFilament(th)}
                        <li role="option" aria-selected={th === destTh}>
                          <button
                            type="button"
                            class="th-menu-item"
                            class:selected={th === destTh}
                            onclick={(e) => {
                              e.stopPropagation();
                              selectToolhead(slot, th);
                            }}
                          >
                            <span
                              class="swatch"
                              style:background={normalizeHex(optFil?.colour ?? "")}
                              title={optFil ? normalizeHex(optFil.colour) : ""}
                            ></span>
                            <span class="th-picker-label">TH{th}</span>
                            {#if optFil?.type}
                              <span class="ftype">{optFil.type}</span>
                            {/if}
                          </button>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              </div>
            </div>
          {/each}
        </div>

        {#if mapComplete && mergeDests.length === 0}
          <div class="map-status ok">All source colors mapped • No merges required</div>
        {:else if mapComplete && mergeDests.length > 0}
          <div class="map-status warn">
            Intentional merge onto TH{mergeDests.join(", TH")} — multiple source slots share a toolhead
          </div>
        {:else}
          <div class="map-status warn">Map every used source slot to a toolhead (1–4)</div>
        {/if}

        {#if usedSlots.some((s) => s > 4)}
          <div class="map-status warn">
            More than 4 source slots — extras must be merged into TH1–4 (never dropped silently)
          </div>
        {/if}
      {/if}
    </div>
  </section>

  <!-- Safety -->
  <section class="safety">
    <div class="safety-item">
      <span class="safety-check">✓</span>
      Original remains unchanged
    </div>
    <div class="safety-item">
      <span class="safety-check">✓</span>
      {#if sourceAnalysis?.hasGcode}
        Embedded plate G-code will be removed
      {:else if sourceAnalysis}
        No embedded plate G-code detected
      {:else}
        G-code status after analyze
      {/if}
    </div>
    <div class="safety-item">
      <span class="safety-check">✓</span>
      Output must be re-sliced in Wonderprint-Orca
    </div>
  </section>

  <!-- Output / Convert -->
  <section class="output-row">
    <div class="output-opts">
      <button type="button" class="btn-secondary details-btn" onclick={() => (showDetails = true)} disabled={!sourceAnalysis && !report}>
        Conversion details
      </button>
      <label class="check-opt" title="Write a markdown conversion report beside the output">
        <input type="checkbox" bind:checked={writeReport} disabled={converting} />
        Write conversion report
      </label>
      <label
        class="check-opt"
        title="When unchecked (default), toolhead colours stay as your Wonderprint template loadout. When checked, MakerWorld source colours are copied onto the toolheads."
      >
        <input type="checkbox" bind:checked={copySourceColours} disabled={converting} />
        Copy source colours onto toolheads
      </label>
    </div>

    <div class="output-field">
      <label class="output-label" for="out-path">Output filename</label>
      <div class="output-input-row">
        <input
          id="out-path"
          type="text"
          data-testid="output-path"
          bind:value={outputPath}
          placeholder="project-zr-ultra-s.3mf"
          disabled={converting}
          spellcheck="false"
        />
        <button type="button" class="btn-secondary" onclick={pickOutputDir} disabled={converting || !sourcePath}>
          Browse
        </button>
      </div>
    </div>

    <p class="convert-hint">Use <strong>Convert project</strong> in the left panel when ready.</p>
  </section>

  {#if phase === "success" && report}
    <section class="result success" data-testid="success-panel">
      <div class="result-title">Conversion complete</div>
      <p class="result-body">
        Output written. Open the project in Wonderprint-Orca and <strong>re-slice</strong> before printing.
        The original source file was not modified.
      </p>
      <div class="result-path" title={report.output}>{report.output}</div>
      {#if report.reportPath}
        <div class="result-path muted" title={report.reportPath}>Report: {report.reportPath}</div>
      {/if}
      <div class="result-actions">
        <button type="button" class="btn-secondary" data-testid="open-folder-btn" onclick={openFolder}>Open folder</button>
        <button type="button" class="btn-secondary" data-testid="copy-path-btn" onclick={copyOutputPath}>{copyPathLabel}</button>
        {#if report.reportPath}
          <button type="button" class="btn-secondary" onclick={openReport}>View report folder</button>
        {/if}
        <button type="button" class="btn-secondary" onclick={convertAnother}>Convert another</button>
      </div>
    </section>
  {/if}

  {#if phase === "error" && convertError}
    <section class="result error">
      <div class="result-title">Conversion failed</div>
      <p class="result-body">
        {convertError}. The original source file was not modified.
      </p>
      {#if convertErrorDetail}
        <button type="button" class="link-btn" onclick={() => (showErrorDetail = !showErrorDetail)}>
          {showErrorDetail ? "Hide technical details" : "Technical details"}
        </button>
        {#if showErrorDetail}
          <pre class="error-detail">{convertErrorDetail}</pre>
        {/if}
      {/if}
      <div class="result-actions">
        <button type="button" class="btn-secondary" onclick={() => (convertError = null)}>Dismiss</button>
      </div>
    </section>
  {/if}
</div>

{#if showDetails}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="modal-backdrop" onclick={() => (showDetails = false)} role="presentation">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions a11y_interactive_supports_focus -->
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="details-title"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="modal-head">
        <h3 id="details-title">Conversion details</h3>
        <button type="button" class="win-x" onclick={() => (showDetails = false)} aria-label="Close">×</button>
      </div>
      <div class="modal-body">
        <dl class="detail-dl">
          <dt>Strategy</dt>
          <dd>{report?.strategy ?? "S1 settings graft (auto)"}</dd>
          <dt>Source printer</dt>
          <dd>{report?.sourcePrinter ?? sourceAnalysis?.printerModel ?? "—"}</dd>
          <dt>Target printer</dt>
          <dd>{report?.outputPrinter ?? templateAnalysis?.printerModel ?? "—"}</dd>
          <dt>Colours patched</dt>
          <dd>{report ? (report.coloursPatched ? "Yes" : "No") : "On convert"}</dd>
          <dt>Slot map</dt>
          <dd>
            {#if report}
              {report.slotMapIdentity
                ? "Identity"
                : report.slotMapPairs.map(([a, b]) => `${a}→${b}`).join(", ") || "—"}
            {:else if sourceAnalysis}
              {buildSlotMapString(slotDest, usedSlots) || "—"}
            {:else}
              —
            {/if}
          </dd>
          <dt>G-code stripped</dt>
          <dd>
            {#if report}
              {report.hadGcodeStripped ? "Yes" : "No"}
            {:else if sourceAnalysis}
              {sourceAnalysis.hasGcode ? "Will remove if present" : "None detected"}
            {:else}
              —
            {/if}
          </dd>
          <dt>Stripped members</dt>
          <dd>{report?.strippedMembers?.join(", ") || "—"}</dd>
          <dt>Paint attrs</dt>
          <dd>
            {#if report}
              {report.paintAttrsRewritten} rewritten / {report.paintAttrsSeen} seen
            {:else if sourceAnalysis}
              {sourceAnalysis.paintColorCount} paint_color attrs
            {:else}
              —
            {/if}
          </dd>
          <dt>Bed</dt>
          <dd>
            {formatBed(sourceAnalysis?.bedSizeMm)} → {formatBed(templateAnalysis?.bedSizeMm)}
          </dd>
          <dt>Source path</dt>
          <dd class="mono">{report?.source ?? sourcePath ?? "—"}</dd>
          <dt>Template path</dt>
          <dd class="mono">{report?.template ?? templatePath ?? "—"}</dd>
          <dt>Output path</dt>
          <dd class="mono">{report?.output ?? (outputPath || "—")}</dd>
          <dt>Report path</dt>
          <dd class="mono">{report?.reportPath ?? "—"}</dd>
          {#if report?.warnings?.length}
            <dt>Warnings</dt>
            <dd>{report.warnings.join("; ")}</dd>
          {/if}
        </dl>
      </div>
    </div>
  </div>
{/if}

{#if plateLightbox}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div
    class="modal-backdrop plate-lightbox-backdrop"
    onclick={() => (plateLightbox = null)}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions a11y_interactive_supports_focus -->
    <div
      class="plate-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label="Plate {plateLightbox.plateIndex} preview"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="modal-head">
        <h3>Plate {plateLightbox.plateIndex}</h3>
        <button type="button" class="win-x" onclick={() => (plateLightbox = null)} aria-label="Close">×</button>
      </div>
      <div class="plate-lightbox-body">
        <img src={plateLightbox.dataUrl} alt="Plate {plateLightbox.plateIndex} full preview" />
      </div>
    </div>
  </div>
{/if}

<style>
  .convert-page {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 1400px;
  }

  .page-heading {
    min-height: 64px;
    margin-top: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
  }

  .page-heading h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 700;
    line-height: 38px;
    color: var(--text-title);
  }

  .subtitle {
    margin: 0;
    font-size: 16px;
    color: var(--text-subtitle);
  }

  .file-cards {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }

  .file-card {
    background: var(--card);
    border: 1px solid var(--border-card);
    border-radius: var(--radius-card);
    padding: 24px 24px 0;
    min-height: 180px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.18);
    transition: border-color 0.12s ease;
  }

  .file-card.drag-over {
    border-color: var(--cyan);
    border-style: dashed;
    box-shadow: 0 0 0 1px var(--cyan);
  }

  .card-label {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.7px;
    color: var(--text-label);
    margin-bottom: 14px;
  }

  .card-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding-bottom: 12px;
  }

  .empty-title {
    font-size: 15px;
    color: var(--text-secondary);
  }

  .card-loaded {
    flex: 1;
    display: flex;
    align-items: flex-start;
    gap: 14px;
    min-width: 0;
  }

  .card-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .card-filename {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-path {
    font-size: 13px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    display: inline-block;
    align-self: flex-start;
    margin-top: 6px;
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
  }

  .badge.cyan {
    background: var(--cyan-surface);
    color: var(--cyan);
    border: 1px solid #1a6a72;
  }

  .badge.green {
    background: var(--green-surface);
    color: var(--green);
    border: 1px solid var(--green-border);
  }

  .drop-strip {
    margin: 12px -24px 0;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    color: var(--text-muted);
    border-top: 1px dashed var(--border);
    background: rgba(0, 0, 0, 0.12);
    border-radius: 0 0 var(--radius-card) var(--radius-card);
  }

  .inline-error {
    color: var(--red);
    font-size: 13px;
    margin-top: 8px;
  }

  .status-strip {
    min-height: 50px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    padding: 10px 16px;
    display: flex;
    flex-direction: column;
    justify-content: center;
  }

  .status-strip.ok {
    border-color: #2a5a35;
  }

  .status-strip.error {
    background: var(--red-surface);
    border-color: #7a2a30;
  }

  .status-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }

  .status-msg {
    font-size: 19px;
    font-weight: 650;
  }

  .status-msg.ok {
    color: var(--green-bright);
  }

  .status-msg.err {
    color: var(--red);
    font-size: 15px;
  }

  .status-msg.analyzing {
    color: var(--text-secondary);
    font-size: 16px;
  }

  .status-msg.muted {
    color: var(--text-muted);
    font-size: 15px;
    font-weight: 500;
  }

  .status-icon.ok {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--green-surface);
    color: var(--green-bright);
    display: grid;
    place-items: center;
    font-size: 13px;
    font-weight: 700;
  }

  .status-icon.err {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: var(--red-surface);
    color: var(--red);
    display: grid;
    place-items: center;
    font-weight: 700;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-left: auto;
  }

  .analysis-warnings {
    margin: 10px 0 0;
    padding: 8px 12px 8px 28px;
    list-style: disc;
    background: var(--amber-surface);
    border: 1px solid var(--amber-border);
    border-radius: 6px;
    color: var(--amber-text);
    font-size: 13px;
    line-height: 1.45;
  }

  .analysis-warnings li {
    margin: 2px 0;
  }

  .chip {
    height: 36px;
    display: inline-flex;
    align-items: center;
    padding: 0 12px;
    background: var(--chip-bg);
    border: 1px solid var(--chip-border);
    border-radius: 6px;
    font-size: 13px;
    color: var(--text-secondary);
  }

  .mid-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(420px, 1fr);
    gap: 10px;
    min-height: 300px;
  }

  .mid-grid.single {
    grid-template-columns: 1fr;
    min-height: 0;
  }

  .panel {
    background: var(--card);
    border: 1px solid var(--border-card);
    border-radius: var(--radius-card);
    padding: 18px 20px 20px;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.18);
    min-width: 0;
  }

  .panel-heading {
    margin: 0 0 14px;
    font-size: 22px;
    font-weight: 650;
    color: var(--text);
  }

  .meta-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    margin-bottom: 14px;
  }

  .meta-table th {
    text-align: left;
    width: 38%;
    padding: 7px 8px 7px 0;
    color: var(--text-muted);
    font-weight: 500;
    border-bottom: 1px solid #1a2e3a;
  }

  .meta-table td {
    padding: 7px 0;
    color: var(--text-secondary);
    border-bottom: 1px solid #1a2e3a;
  }

  .arrow {
    margin: 0 6px;
    color: var(--text-muted);
  }

  .cyan-text {
    color: var(--cyan);
  }

  .plate-summaries {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-bottom: 10px;
  }

  .plate-card {
    width: 148px;
    height: 108px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    overflow: hidden;
    position: relative;
  }

  .plate-card.has-thumb {
    justify-content: flex-end;
  }

  .plate-card.clickable {
    cursor: zoom-in;
    padding: 0;
    font: inherit;
    color: inherit;
  }

  .plate-card.clickable:hover {
    border-color: var(--cyan);
    box-shadow: 0 0 0 1px var(--cyan);
  }

  .plate-card.clickable:focus-visible {
    outline: 2px solid var(--cyan);
    outline-offset: 2px;
  }

  .plate-thumb {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    background: #0a1218;
  }

  .plate-lightbox-backdrop {
    z-index: 1200;
  }

  .plate-lightbox {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 10px;
    max-width: min(920px, 94vw);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }

  .plate-lightbox-body {
    padding: 12px 16px 16px;
    overflow: auto;
    display: flex;
    justify-content: center;
    align-items: center;
    background: #050d14;
  }

  .plate-lightbox-body img {
    max-width: 100%;
    max-height: min(72vh, 720px);
    object-fit: contain;
    border-radius: 4px;
  }

  .plate-num {
    font-size: 13px;
    font-weight: 600;
    z-index: 1;
  }

  .plate-num.overlay {
    width: 100%;
    padding: 4px 6px;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.72));
    font-size: 12px;
    text-align: center;
  }

  .plate-sub {
    font-size: 11px;
    color: var(--text-muted);
  }

  .th-legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
    padding: 8px 10px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .th-legend-label {
    font-size: 12px;
    font-weight: 650;
    color: var(--text-muted);
    margin-right: 4px;
  }

  .th-legend-item {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .th-legend-th {
    font-weight: 600;
  }

  .swatch.sm {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid var(--border-strong);
    flex-shrink: 0;
  }

  .muted-help.sm {
    font-size: 12px;
    margin-bottom: 8px;
  }

  .th-box {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    position: relative;
  }

  .th-picker {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .th-picker-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 40px;
    padding: 6px 10px;
    background: var(--input);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text);
    font: inherit;
    font-size: 14px;
    cursor: pointer;
    text-align: left;
  }

  .th-picker-btn:hover:not(:disabled) {
    border-color: var(--cyan);
  }

  .th-picker-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .th-picker-btn:focus-visible {
    outline: 2px solid var(--cyan);
    outline-offset: 1px;
  }

  .th-picker-label {
    font-weight: 650;
    min-width: 2.4em;
  }

  .th-caret {
    margin-left: auto;
    color: var(--text-muted);
    font-size: 12px;
  }

  .th-menu {
    position: absolute;
    left: 0;
    right: 0;
    top: calc(100% + 4px);
    z-index: 40;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  }

  .th-menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 10px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 14px;
    cursor: pointer;
    text-align: left;
  }

  .th-menu-item:hover {
    background: var(--cyan-surface);
  }

  .th-menu-item.selected {
    background: var(--cyan-surface);
    outline: 1px solid var(--cyan);
  }

  .muted-box {
    color: var(--text-muted);
    font-size: 13px;
    padding: 12px;
    border: 1px dashed var(--border);
    border-radius: 6px;
  }

  .bed-warn {
    margin-top: 10px;
    padding: 10px 12px;
    background: var(--amber-surface);
    border: 1px solid var(--amber-border);
    border-radius: 6px;
    color: var(--amber-text);
    font-size: 13px;
    line-height: 1.4;
  }

  .map-rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .map-rows.disabled {
    opacity: 0.6;
    pointer-events: none;
  }

  .map-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 50px;
    padding: 6px 10px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .src-box {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .slot-label {
    font-size: 13px;
    font-weight: 600;
    min-width: 52px;
  }

  .swatch {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: 1px solid var(--border-strong);
    flex-shrink: 0;
  }

  .hex {
    font-size: 13px;
    font-family: ui-monospace, Consolas, monospace;
    color: var(--text-secondary);
  }

  .ftype {
    font-size: 12px;
    color: var(--text-muted);
  }

  .map-arrow {
    color: var(--text-muted);
  }

  .th-select {
    min-width: 88px;
    height: 36px;
    background: var(--input);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    padding: 0 8px;
    color: var(--text);
  }

  .map-status {
    margin-top: 12px;
    font-size: 13px;
    padding: 8px 10px;
    border-radius: 6px;
  }

  .map-status.ok {
    background: var(--green-surface);
    border: 1px solid var(--green-border);
    color: var(--green);
  }

  .map-status.warn {
    background: var(--amber-surface);
    border: 1px solid var(--amber-border);
    color: var(--amber-text);
  }

  .muted-help {
    color: var(--text-muted);
    font-size: 14px;
  }

  .safety {
    min-height: 62px;
    display: flex;
    flex-wrap: wrap;
    gap: 12px 28px;
    align-items: center;
    padding: 14px 18px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
  }

  .safety-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    color: var(--text-secondary);
  }

  .safety-check {
    color: var(--green);
    font-weight: 700;
  }

  .output-row {
    min-height: 88px;
    display: flex;
    align-items: flex-end;
    gap: 14px;
    flex-wrap: wrap;
    padding: 12px 0 4px;
  }

  .convert-hint {
    margin: 0 0 6px;
    font-size: 13px;
    color: var(--text-muted);
    align-self: center;
    max-width: 220px;
  }

  .convert-hint strong {
    color: var(--cyan);
    font-weight: 650;
  }

  .output-opts {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-self: center;
    min-width: 200px;
  }

  .details-btn {
    height: 44px;
    align-self: flex-start;
  }

  .check-opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
  }

  .check-opt input {
    accent-color: var(--cyan-dark);
    width: 15px;
    height: 15px;
  }

  .output-field {
    flex: 1;
    min-width: 280px;
    max-width: 520px;
  }

  .output-label {
    display: block;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.4px;
    color: var(--text-label);
    margin-bottom: 6px;
    text-transform: uppercase;
  }

  .output-input-row {
    display: flex;
    gap: 8px;
  }

  .output-input-row input {
    flex: 1;
    height: 48px;
    background: var(--input);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    padding: 0 12px;
    font-size: 15px;
    color: var(--text);
    user-select: text;
  }

  .output-input-row input:focus {
    outline: 1px solid var(--cyan-dark);
    border-color: var(--cyan-dark);
  }

  .btn-secondary {
    height: 40px;
    padding: 0 14px;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    color: var(--text-secondary);
    font-size: 14px;
    font-weight: 500;
    white-space: nowrap;
  }

  .btn-secondary:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }

  .btn-convert {
    min-width: 240px;
    width: 295px;
    max-width: 100%;
    height: 70px;
    background: var(--cyan-btn);
    border: 1px solid var(--cyan-btn-border);
    border-radius: 8px;
    color: #fff;
    font-size: 21px;
    font-weight: 650;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    margin-left: auto;
  }

  .btn-convert:hover:not(:disabled) {
    background: var(--cyan-btn-hover);
  }

  .btn-convert:active:not(:disabled) {
    background: var(--cyan-btn-pressed);
  }

  .result {
    margin-top: 4px;
    padding: 18px 20px;
    border-radius: var(--radius-card);
    border: 1px solid var(--border);
  }

  .result.success {
    background: var(--green-surface);
    border-color: var(--green-border);
  }

  .result.error {
    background: var(--red-surface);
    border-color: #7a2a30;
  }

  .result-title {
    font-size: 18px;
    font-weight: 650;
    margin-bottom: 8px;
  }

  .result.success .result-title {
    color: var(--green-bright);
  }

  .result.error .result-title {
    color: var(--red);
  }

  .result-body {
    margin: 0 0 10px;
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.45;
  }

  .result-path {
    font-size: 13px;
    font-family: ui-monospace, Consolas, monospace;
    color: var(--text);
    word-break: break-all;
    margin-bottom: 6px;
    user-select: text;
  }

  .result-path.muted {
    color: var(--text-muted);
  }

  .result-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 12px;
  }

  .link-btn {
    background: none;
    border: none;
    color: var(--cyan);
    font-size: 13px;
    padding: 0;
    text-decoration: underline;
  }

  .error-detail {
    margin: 8px 0 0;
    padding: 10px;
    background: rgba(0, 0, 0, 0.25);
    border-radius: 6px;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    max-height: 160px;
    overflow: auto;
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: grid;
    place-items: center;
    z-index: 100;
    padding: 24px;
  }

  .modal {
    width: min(560px, 100%);
    max-height: min(80vh, 720px);
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border-strong);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.45);
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }

  .modal-head h3 {
    margin: 0;
    font-size: 18px;
  }

  .win-x {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 22px;
    line-height: 1;
    padding: 4px 8px;
  }

  .win-x:hover {
    color: var(--text);
  }

  .modal-body {
    padding: 14px 16px 20px;
  }

  .detail-dl {
    display: grid;
    grid-template-columns: 140px 1fr;
    gap: 8px 12px;
    margin: 0;
    font-size: 13px;
  }

  .detail-dl dt {
    color: var(--text-muted);
    margin: 0;
  }

  .detail-dl dd {
    margin: 0;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .detail-dl dd.mono {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 12px;
    user-select: text;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    border: 0;
  }

  @media (max-width: 1000px) {
    .file-cards {
      grid-template-columns: 1fr;
    }
    .mid-grid {
      grid-template-columns: 1fr;
    }
    .btn-convert {
      width: 100%;
      margin-left: 0;
    }
  }
</style>
