<script lang="ts">
  // Help content — template export, open Wonderprint, re-slice, local only
</script>

<div class="help-page" data-testid="help-page">
  <header class="page-heading">
    <h1>Help</h1>
    <p class="subtitle">
      How 3MF Profile Transplant works and how to finish a conversion safely.
    </p>
  </header>

  <section class="help-card">
    <h2>What this app does</h2>
    <p>
      <strong>3MF Profile Transplant</strong> takes a MakerWorld / Bambu Studio project
      <code>.3mf</code> and grafts Wonderprint-Orca (ZR Ultra-S) printer settings from a template
      you provide. Geometry, plates, part colors, and paint metadata are preserved. Conversion
      runs <strong>entirely on your PC</strong> — no model upload, no cloud account.
    </p>
    <ul>
      <li>Maps source filament slots to ZR Ultra-S toolheads TH1–TH4</li>
      <li>Replaces incompatible project settings from your Wonderprint template</li>
      <li>Strips embedded plate G-code and other unsafe slice artifacts</li>
      <li>Writes a conversion report next to the output package</li>
      <li><strong>Never overwrites</strong> the original source <code>.3mf</code></li>
    </ul>
  </section>

  <section class="help-card">
    <h2>Export a Wonderprint template</h2>
    <ol>
      <li>Open <strong>Wonderprint-Orca</strong> (or your ZR Ultra-S profile pack).</li>
      <li>Create or open a simple project using the target printer profile (e.g. ZR Ultra-S, 0.4&nbsp;mm).</li>
      <li>Save / export as a project <code>.3mf</code> (settings donor). A clean template such as
        <code>WonderClean.3mf</code> works well.</li>
      <li>In this app, choose that file under <strong>Wonderprint template</strong>. The last path is remembered.</li>
    </ol>
    <p class="note">
      The template supplies printer/process/G-code settings only. Your source geometry is kept from
      the Bambu/MakerWorld project (S1 graft). Geometry-only packages may use S2 shell inject automatically.
    </p>
  </section>

  <section class="help-card">
    <h2>Convert workflow</h2>
    <ol>
      <li>Select the <strong>source project</strong> (Bambu / MakerWorld <code>.3mf</code>).</li>
      <li>Select the <strong>Wonderprint template</strong>.</li>
      <li>Review analysis (printer, plates, colors, bed size).</li>
      <li>Map each used source slot to TH1–TH4. Merges are allowed and shown explicitly; slots are never dropped silently.</li>
      <li>Confirm the output path (default <code>{`{stem}`}-zr-ultra-s.3mf</code> beside the source).</li>
      <li>Review project analysis in the <strong>left panel</strong>, then click <strong>Convert project</strong> there (under Help).</li>
    </ol>
  </section>

  <section class="help-card">
    <h2>Open in Wonderprint and re-slice</h2>
    <p>
      After conversion, open the output <code>.3mf</code> in <strong>Wonderprint-Orca</strong>,
      verify placement on the bed, check toolhead assignments, and <strong>slice again</strong>.
    </p>
    <p class="warn">
      Converted projects are <strong>not ready to print</strong> until you re-slice. This app does
      not convert Bambu G-code into machine G-code for independent toolheads — it produces an
      editable project with transplanted profiles.
    </p>
  </section>

  <section class="help-card">
    <h2>Safety guarantees</h2>
    <ul>
      <li>Original source file is left unchanged</li>
      <li>Output path must differ from source and template</li>
      <li>Embedded plate G-code is stripped when present</li>
      <li>No network permission is required for convert</li>
      <li>Local-only indicator remains visible in the sidebar</li>
    </ul>
  </section>

  <section class="help-card">
    <h2>CLI (automation)</h2>
    <p>
      The same core is available as <code>wondermaker_3mf_cli</code> for scripts and CI. The desktop
      app is a thin shell over the same in-process library — no conversion daemon.
    </p>
  </section>
</div>

<style>
  .help-page {
    max-width: 820px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .page-heading {
    margin-top: 4px;
    margin-bottom: 8px;
  }

  .page-heading h1 {
    margin: 0 0 8px;
    font-size: 36px;
    font-weight: 700;
    line-height: 44px;
    color: var(--text-title);
  }

  .subtitle {
    margin: 0;
    font-size: 16px;
    color: var(--text-subtitle);
  }

  .help-card {
    background: var(--card);
    border: 1px solid var(--border-card);
    border-radius: var(--radius-card);
    padding: 20px 22px;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.18);
  }

  .help-card h2 {
    margin: 0 0 12px;
    font-size: 20px;
    font-weight: 650;
  }

  .help-card p,
  .help-card li {
    font-size: 15px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .help-card p {
    margin: 0 0 10px;
  }

  .help-card ul,
  .help-card ol {
    margin: 0 0 8px;
    padding-left: 1.35rem;
  }

  .help-card li {
    margin-bottom: 6px;
  }

  .help-card code {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.92em;
    background: var(--surface-1);
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid var(--border);
    color: var(--cyan);
  }

  .note {
    margin-top: 10px !important;
    padding: 10px 12px;
    background: var(--surface-1);
    border-radius: 6px;
    border-left: 3px solid var(--cyan);
  }

  .warn {
    margin-top: 10px !important;
    padding: 10px 12px;
    background: var(--amber-surface);
    border: 1px solid var(--amber-border);
    border-radius: 6px;
    color: var(--amber-text) !important;
  }
</style>
