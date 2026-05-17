export interface MxlItemDetail {
  name: string;
  nameDisplay: string;
  quality: string;
  class: string;
  typeName: string;
  runeword: string;
  runewordLevel: string;
  stats: string;
}

export interface MxlItemEntry {
  name: string;
  quality: string;
  class: string;
  typeName: string;
  detail: MxlItemDetail | null;
}

export type MxlItemSearchResult =
  | { kind: 'results'; query: string; entries: MxlItemEntry[]; message: string | null }
  | { kind: 'notFound'; query: string; message: string }
  | { kind: 'rateLimited'; message: string; retryAfterMs: number }
  | { kind: 'error'; query: string; message: string };

export interface OpenItemSearchPayload {
  query?: string | null;
}

const qualityColors: Record<string, string> = {
  TU: 'var(--quality-unique)',
  SU: 'var(--quality-unique)',
  SSU: 'var(--quality-unique)',
  SSSU: 'var(--quality-unique)',
  Unique: 'var(--quality-unique)',
  Set: 'var(--quality-set)',
  'Sacred Set': 'var(--quality-set)',
  Quest: 'var(--quality-crafted)',
  Rare: 'var(--quality-rare)',
  Magic: 'var(--quality-magic)',
  Crafted: 'var(--quality-crafted)',
};

export function itemQualityColor(quality: string): string {
  return qualityColors[quality] ?? '#f0f0f0';
}
