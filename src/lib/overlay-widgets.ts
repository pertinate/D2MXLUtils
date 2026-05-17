/**
 * Static registry of overlay positions and edit-mode repositionable widgets.
 *
 * Adding a new persisted overlay position:
 *   1. Add a default to OVERLAY_POSITION_DEFAULTS below.
 *   2. In the widget's component, read its position via
 *      `widgetPosition(id)` from `src/stores/widget-positions.svelte.ts`.
 *   3. Style with `top: {y}%; left: {x}%;` (percent of overlay size).
 *   4. Add to OVERLAY_WIDGETS only if it should appear in overlay edit mode.
 */

export const OVERLAY_POSITION_DEFAULTS = {
  notifications: { x: 1, y: 1 },
  'dps-meter': { x: 1, y: 1 },
  'loot-history': { x: 50, y: 25 },
  'item-search': { x: 30, y: 16 },
} as const;

export type OverlayPositionId = keyof typeof OVERLAY_POSITION_DEFAULTS;

export interface OverlayWidgetSpec {
  /** Settings key — NEVER change after release. */
  id: OverlayPositionId;
  label: string;
  /** Pixels. Sizes the ghost and clamps drag. */
  ghostSize: { width: number; height: number };
}

export const OVERLAY_WIDGETS = [
  {
    id: 'notifications',
    label: 'Drop notifications',
    ghostSize: { width: 300, height: 80 },
  },
  {
    id: 'dps-meter',
    label: 'DPS meter',
    ghostSize: { width: 130, height: 110 },
  },
] as const satisfies readonly OverlayWidgetSpec[];

export type OverlayWidgetId = (typeof OVERLAY_WIDGETS)[number]['id'];
