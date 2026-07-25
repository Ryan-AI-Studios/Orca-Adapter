import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisDto,
  AppConfigDto,
  ConversionReportDto,
  ConvertDto,
  PlateThumbnailDto,
} from "./types";

export function analyze3mf(sourcePath: string): Promise<AnalysisDto> {
  return invoke("analyze_3mf", { sourcePath });
}

export function validateTemplate(templatePath: string): Promise<AnalysisDto> {
  return invoke("validate_template", { templatePath });
}

/** Plate previews from Metadata/plate_N.png (etc.) as data URLs. */
export function extractPlateThumbnails(
  sourcePath: string,
  maxPlates: number,
): Promise<PlateThumbnailDto[]> {
  return invoke("extract_plate_thumbnails", { sourcePath, maxPlates });
}

export function convert3mf(opts: ConvertDto): Promise<ConversionReportDto> {
  return invoke("convert_3mf", { opts });
}

export function openOutputFolder(path: string): Promise<void> {
  return invoke("open_output_folder", { path });
}

export function getConfig(): Promise<AppConfigDto> {
  return invoke("get_config");
}

export function setTemplatePath(templatePath: string | null): Promise<void> {
  return invoke("set_template_path", { templatePath });
}

export function suggestOutputPath(sourcePath: string): Promise<string> {
  return invoke("suggest_output_path", { sourcePath });
}

export function pathExists(path: string): Promise<boolean> {
  return invoke("path_exists", { path });
}

/**
 * Used source slots that must be mapped.
 * Prefer core `usedSourceSlots` (histogram ∪ paint); fall back for older DTOs.
 */
export function usedSourceSlots(analysis: AnalysisDto): number[] {
  if (Array.isArray(analysis.usedSourceSlots) && analysis.usedSourceSlots.length > 0) {
    return [...analysis.usedSourceSlots]
      .filter((n) => Number.isFinite(n) && n >= 1)
      .sort((a, b) => a - b);
  }
  const keys = Object.keys(analysis.extruderHistogram)
    .map((k) => Number(k))
    .filter((n) => Number.isFinite(n) && n >= 1)
    .sort((a, b) => a - b);
  if (keys.length > 0) return keys;
  if (analysis.filaments.length > 0) {
    return analysis.filaments.map((f) => f.index1based).sort((a, b) => a - b);
  }
  return [1];
}

export function buildSlotMapString(map: Record<number, number>, sources: number[]): string {
  return sources.map((s) => `${s}=${map[s] ?? s}`).join(",");
}

export function formatBytes(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatBed(bed: [number, number] | null | undefined): string {
  if (!bed) return "—";
  const [w, d] = bed;
  return `${Math.round(w)} × ${Math.round(d)} mm`;
}

export function basename(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

export function normalizeHex(colour: string): string {
  const c = colour.trim();
  if (!c) return "#888888";
  if (c.startsWith("#")) {
    if (c.length === 9) return c.slice(0, 7); // drop alpha
    return c;
  }
  if (/^[0-9A-Fa-f]{6}$/.test(c)) return `#${c}`;
  if (/^[0-9A-Fa-f]{8}$/.test(c)) return `#${c.slice(0, 6)}`;
  return "#888888";
}
