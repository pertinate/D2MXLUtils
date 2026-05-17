/**
 * Reactive accessors for centralized widget positions.
 *
 * Reading: `widgetPosition(id)` inside a `$derived` stays reactive
 * because it reads `settingsStore.settings`, which is a `$state`.
 *
 * Writing: `setWidgetPosition` plugs into the existing dirty-keys
 * mechanism, so a write from the overlay window does not get clobbered
 * by a concurrent save from the main window (and vice versa).
 */

import { settingsStore } from './settings.svelte';
import { OVERLAY_POSITION_DEFAULTS, type OverlayPositionId } from '../lib/overlay-widgets';

export function widgetPosition(id: OverlayPositionId): { x: number; y: number } {
  const fallback = OVERLAY_POSITION_DEFAULTS[id];
  return settingsStore.settings.widgetPositions?.[id] ?? { x: fallback.x, y: fallback.y };
}

export function setWidgetPosition(id: OverlayPositionId, x: number, y: number): void {
  settingsStore.set('widgetPositions', {
    ...settingsStore.settings.widgetPositions,
    [id]: { x, y },
  });
}
