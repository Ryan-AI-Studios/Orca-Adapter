/**
 * Bridge between the Home (convert) page and the app shell sidebar.
 * The page publishes analysis + convert actions; the layout renders them under nav.
 */
import type { AnalysisDto, PlateThumbnailDto } from "$lib/types";

export type HomeShellState = {
  /** True while the Home route is mounted and publishing. */
  active: boolean;
  canConvert: boolean;
  converting: boolean;
  analyzing: boolean;
  progressStage: string | null;
  analysisError: string | null;
  bedWarning: string | null;
  sourceAnalysis: AnalysisDto | null;
  templateAnalysis: AnalysisDto | null;
  plateThumbs: Record<number, PlateThumbnailDto>;
  onConvert: (() => void) | null;
  onOpenPlate: ((plateIndex: number) => void) | null;
};

export const homeShell: HomeShellState = $state({
  active: false,
  canConvert: false,
  converting: false,
  analyzing: false,
  progressStage: null,
  analysisError: null,
  bedWarning: null,
  sourceAnalysis: null,
  templateAnalysis: null,
  plateThumbs: {},
  onConvert: null,
  onOpenPlate: null,
});

export function publishHomeShell(
  partial: Partial<Omit<HomeShellState, "active">> & { active?: boolean },
) {
  Object.assign(homeShell, partial);
  if (partial.active !== undefined) {
    homeShell.active = partial.active;
  } else {
    homeShell.active = true;
  }
}

export function clearHomeShell() {
  homeShell.active = false;
  homeShell.canConvert = false;
  homeShell.converting = false;
  homeShell.analyzing = false;
  homeShell.progressStage = null;
  homeShell.analysisError = null;
  homeShell.bedWarning = null;
  homeShell.sourceAnalysis = null;
  homeShell.templateAnalysis = null;
  homeShell.plateThumbs = {};
  homeShell.onConvert = null;
  homeShell.onOpenPlate = null;
}
