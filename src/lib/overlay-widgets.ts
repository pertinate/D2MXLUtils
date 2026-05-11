/**
 * Static registry of every repositionable overlay widget.
 *
 * Adding a new widget:
 *   1. Add an entry to OVERLAY_WIDGETS below.
 *   2. In the widget's component, read its position via
 *      `widgetPosition(id)` from `src/stores/widget-positions.svelte.ts`.
 *   3. Style with `top: {y}%; left: {x}%;` (percent of overlay size).
 */

export interface OverlayWidgetSpec {
  /** Settings key — NEVER change after release. */
  id: string;
  label: string;
  /** Percent. Used when the widget has no saved position yet. */
  defaultPosition: { x: number; y: number };
  /** Pixels. Sizes the ghost and clamps drag. */
  ghostSize: { width: number; height: number };
}

export const OVERLAY_WIDGETS = [
  {
    id: 'notifications',
    label: 'Drop notifications',
    defaultPosition: { x: 1, y: 1 },
    ghostSize: { width: 300, height: 80 },
  },
  {
    id: 'dps-meter',
    label: 'DPS meter',
    defaultPosition: { x: 1, y: 1 },
    ghostSize: { width: 130, height: 110 },
  },
  {
    id: 'loot-history',
    label: 'Loot history',
    defaultPosition: { x: 50, y: 25 },
    ghostSize: { width: 600, height: 400 },
  },
] as const satisfies readonly OverlayWidgetSpec[];

export type OverlayWidgetId = (typeof OVERLAY_WIDGETS)[number]['id'];
