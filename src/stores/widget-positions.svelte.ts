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
import {
  OVERLAY_WIDGETS,
  type OverlayWidgetId,
} from '../lib/overlay-widgets';

const SPECS = new Map(OVERLAY_WIDGETS.map((w) => [w.id, w]));

export function widgetPosition(id: OverlayWidgetId): { x: number; y: number } {
  return (
    settingsStore.settings.widgetPositions?.[id]
    ?? SPECS.get(id)!.defaultPosition
  );
}

export function setWidgetPosition(
  id: OverlayWidgetId,
  x: number,
  y: number,
): void {
  settingsStore.set('widgetPositions', {
    ...settingsStore.settings.widgetPositions,
    [id]: { x, y },
  });
}
