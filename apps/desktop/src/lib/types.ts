export type FilamentDto = {
  index1based: number;
  colour: string;
  type: string;
};

export type AnalysisDto = {
  path: string;
  fileName: string;
  fileSizeBytes: number | null;
  application: string | null;
  printerModel: string | null;
  bedSizeMm: [number, number] | null;
  plateCount: number;
  filaments: FilamentDto[];
  extruderHistogram: Record<string, number>;
  hasPaintColor: boolean;
  paintColorCount: number;
  hasGcode: boolean;
  warnings: string[];
  /** Sorted unique 1-based source slots that must be mapped (histogram ∪ paint). */
  usedSourceSlots: number[];
  colorMode: string;
  coloredParts: number;
  colorCount: number;
};

export type ConvertDto = {
  source: string;
  template: string;
  output: string;
  slotMap: string;
  copyFilamentType?: boolean;
  writeReport?: boolean;
  reportPath?: string | null;
  strictBed?: boolean;
  strategy?: string;
};

export type ConversionReportDto = {
  source: string;
  template: string;
  output: string;
  strategy: string;
  sourcePrinter: string | null;
  outputPrinter: string | null;
  strippedMembers: string[];
  coloursPatched: boolean;
  slotMapIdentity: boolean;
  slotMapPairs: [number, number][];
  paintAttrsSeen: number;
  paintAttrsRewritten: number;
  hadGcodeStripped: boolean;
  reportPath: string | null;
  plates: number | null;
  extruderHistogramOut: Record<string, number>;
  coloursBefore: string[];
  coloursAfter: string[];
  warnings: string[];
  entryCount: number;
  opcReconciled: boolean;
};

export type AppConfigDto = {
  templatePath: string | null;
};

export type ProgressEvent = {
  stage: string;
  index: number;
  total: number;
};

export type FlowPhase =
  | "empty"
  | "source"
  | "template"
  | "analyzing"
  | "ready"
  | "converting"
  | "success"
  | "error";
