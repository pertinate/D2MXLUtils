# Overlay widget repositioning — unified module

**Date:** 2026-05-09
**Status:** Spec, awaiting plan

## Background

The overlay window renders multiple components on top of Diablo II:
`NotificationStack`, `LootHistoryPanel`, `DpsMeter`, and an open-ended set of
future widgets. The user holds an "edit overlay" hotkey (default Ctrl+Alt) to
reposition them.

Today repositioning works only for `NotificationStack`. Earlier attempts to
add it to `LootHistoryPanel` and `DpsMeter` failed because of a layered
combination of issues:

1. `OverlayEditGrid` covers the viewport with `pointer-events: auto` and
   `z-index: 10000`, swallowing every mouse event before any widget under it
   can receive `mousedown`. `DpsMeter` has correct local drag handlers
   (`src/components/DpsMeter.svelte:31-56`) but never sees the click.
2. Each widget that wants drag support has to re-implement the same
   plumbing: edit-mode awareness, click-through coordination via
   `set_overlay_interactive`, persistence, cross-window sync.
3. Coordinate units are inconsistent — notifications use percent of overlay
   size (responsive to game window resize), `DpsMeter` uses pixels.
4. `LootHistoryPanel` is centered with `transform: translate(-50%, -50%)`,
   which is awkward to combine with absolute drag positioning.
5. There is no place to store positions for widgets that are not currently
   visible (notifications appear only on drops, history only on hotkey,
   `DpsMeter` only when enabled), so a "drag the visible widget" pattern
   cannot work for transient or hideable widgets.

## Goals

- A single reusable module that handles drag/persistence/edit-mode for
  every overlay widget, present and future.
- Adding a new repositionable widget should require: one entry in a
  registry, and reading position via a helper. No drag code, no
  edit-mode wiring, no settings boilerplate.
- Hold-the-hotkey UX preserved: a screen-wide grid background indicates
  edit mode, and every widget shows a draggable "ghost" placeholder at
  its current position regardless of whether the real widget is
  currently rendered.
- Coordinate system unified to percent of overlay size. The choice is
  enforced inside the module — widget authors do not pick.
- Widget positions live in a single centralized settings dictionary.
- Existing user data (notification anchor) is migrated transparently.
- Migration logic lives in an isolated, scalable folder so future
  schema changes follow the same pattern.

## Non-goals

- Snap to grid, alignment guides, multi-select drag.
- Resize handles. Widget sizes stay fixed by their CSS.
- Animated transition when a widget snaps to a newly committed position.
- Per-monitor or per-resolution position profiles.

## Architecture

### Components and ownership

```
Rust hotkeys.rs (EditModeState)
  emits 'overlay-edit-mode' { active: bool }
        │
        ▼
OverlayWindow.svelte
  toggles editActive; calls set_overlay_interactive(active || historyVisible);
  renders <OverlayEditGrid /> when editActive
        │
        ▼
OverlayEditGrid.svelte
  iterates OVERLAY_WIDGETS registry
  renders one <DragGhost> per widget
  owns local "pending" positions (dragged in real time)
  commits to settings on mouseup
        │
        ▼
widget-positions store
  reads/writes settings.widgetPositions[id]
        │
        ▼
NotificationStack / DpsMeter / LootHistoryPanel
  each calls widgetPosition(id) to read position
  styles itself with top/left percent
  knows nothing about edit mode, drag, or persistence
```

### The bug fix that unlocks everything

`OverlayEditGrid` background changes from `pointer-events: auto` to
`pointer-events: none`. Only `<DragGhost>` children opt back into pointer
events. The grid pattern stays purely visual; ghosts capture clicks
directly. This automatically resolves the DPS meter regression and removes
the blocker that broke earlier loot-history attempts.

## Settings shape

### `AppSettings` additions and removals

```ts
export interface WidgetPosition {
  x: number; // percent of overlay width, 0..100
  y: number; // percent of overlay height, 0..100
}

export interface AppSettings {
  // ... existing fields, MINUS:
  //   notificationX, notificationY  ← removed
  //   dpsMeter.position              ← removed (DPS meter unreleased)
  widgetPositions: Record<string, WidgetPosition>;
}
```

The Rust mirror in `src-tauri/src/settings.rs` matches:

```rust
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct WidgetPosition { pub x: f64, pub y: f64 }

pub struct AppSettings {
    // ... existing fields, MINUS notification_x/y and dps_meter.position
    #[serde(default)]
    pub widget_positions: HashMap<String, WidgetPosition>,
}
```

`AppSettings` carries no legacy field names. Anything legacy lives only
inside `migrations/`.

### Migration folder

```
src-tauri/src/migrations/
├── mod.rs                            ← orchestrator
└── v1_24_widget_positions.rs         ← one migration, one file
```

`mod.rs`:

```rust
//! Settings migrations applied at load time.
//!
//! Adding a migration:
//!   1. Create migrations/v<version>_<topic>.rs with a single
//!      pub fn apply(raw: &Value, s: &mut AppSettings) -> bool.
//!   2. Add the module + a call below.
//!
//! Each migration must be idempotent (gate on the new field's
//! presence). After migrate() returns true, settings.rs re-saves
//! to disk, so legacy keys disappear on the next load.

mod v1_24_widget_positions;

use serde_json::Value;
use crate::settings::AppSettings;

pub fn migrate(raw: &Value, s: &mut AppSettings) -> bool {
    let mut changed = false;
    changed |= v1_24_widget_positions::apply(raw, s);
    changed
}
```

`v1_24_widget_positions.rs`:

```rust
//! v1.23 → v1.24: top-level `notificationX/Y` (percent) moved into
//! widget_positions["notifications"]. Done as part of the unified
//! overlay-widget-repositioning module.

use serde::Deserialize;
use serde_json::Value;
use crate::settings::{AppSettings, WidgetPosition};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")] // matches AppSettings's wire format
struct LegacyKeys {
    #[serde(default)]
    notification_x: Option<f64>,
    #[serde(default)]
    notification_y: Option<f64>,
}

pub fn apply(raw: &Value, s: &mut AppSettings) -> bool {
    if s.widget_positions.contains_key("notifications") {
        return false;
    }
    let legacy: LegacyKeys = serde_json::from_value(raw.clone())
        .unwrap_or_default();
    // Skip when neither legacy key was present (fresh install): the
    // helper's spec default kicks in and we avoid writing a noisy
    // pre-populated entry to settings.
    if legacy.notification_x.is_none() && legacy.notification_y.is_none() {
        return false;
    }
    let x = legacy.notification_x.unwrap_or(1.0);
    let y = legacy.notification_y.unwrap_or(1.0);
    s.widget_positions.insert(
        "notifications".into(),
        WidgetPosition { x, y },
    );
    true
}
```

`settings.rs::load_settings` (existing Tauri command, currently at
`src-tauri/src/settings.rs:262-281`) gains a migration step. Storage is
through `tauri-plugin-store`, not direct file IO:

```rust
#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<AppSettings, String> {
    let store = app.store(SETTINGS_FILE)
        .map_err(|e| format!("Failed to open settings store: {}", e))?;

    let Some(raw) = store.get("settings") else {
        return Ok(AppSettings::default()); // fresh install — nothing to migrate
    };

    let mut s: AppSettings = serde_json::from_value(raw.clone())
        .unwrap_or_else(|e| {
            log_error(&format!("Failed to parse settings, using defaults: {}", e));
            AppSettings::default()
        });

    if migrations::migrate(&raw, &mut s) {
        // Re-serialize without legacy keys and persist.
        let value = serde_json::to_value(&s)
            .map_err(|e| format!("Failed to serialize migrated settings: {}", e))?;
        store.set("settings", value);
        store.save()
            .map_err(|e| format!("Failed to save migrated settings: {}", e))?;
    }
    Ok(s)
}
```

Properties:

- One file per migration. The history of settings schema changes reads
  as `ls migrations/`.
- Removing a migration in the future = delete one file + one line in
  `mod.rs`. Nothing else to clean up.
- `AppSettings` never references legacy field names anywhere in the
  codebase outside the migration file.
- Legacy keys disappear from `settings.json` on the first save after
  upgrade.

### DPS meter position

The DPS meter is not in any release yet (introduced on master in commit
`a613a50`, no release tag since). Its `dps_meter.position` field is simply
removed without migration. Default position comes from the registry.

### Cross-window sync

`widgetPositions` plugs into the existing `_dirtyKeys` flow in
`SettingsStore` (`src/stores/settings.svelte.ts:168-230`) without changes
to the merge logic. When the overlay commits a drag while the main window
is open, the dirty-key marker on `widgetPositions` ensures the overlay's
write does not get clobbered by the main window's stale save.

## API surface

### Registry — `src/lib/overlay-widgets.ts`

```ts
export interface OverlayWidgetSpec {
  /** Stable id used as settings key. NEVER change after release. */
  id: string;
  /** Shown on the ghost label during edit mode. */
  label: string;
  /** Used when the widget has no saved position yet. Percent. */
  defaultPosition: { x: number; y: number };
  /** Approximate rendered size in pixels. Sizes the ghost and clamps drag. */
  ghostSize: { width: number; height: number };
}

export const OVERLAY_WIDGETS = [
  { id: 'notifications', label: 'Drop notifications',
    defaultPosition: { x: 1, y: 1 },   ghostSize: { width: 300, height: 80 } },
  { id: 'dps-meter',     label: 'DPS meter',
    defaultPosition: { x: 1, y: 1 },   ghostSize: { width: 130, height: 110 } },
  { id: 'loot-history',  label: 'Loot history',
    defaultPosition: { x: 50, y: 25 }, ghostSize: { width: 600, height: 400 } },
] as const satisfies readonly OverlayWidgetSpec[];

export type OverlayWidgetId = typeof OVERLAY_WIDGETS[number]['id'];
```

Adding a new widget = one entry in this array.

### Helper — `src/stores/widget-positions.svelte.ts`

```ts
import { settingsStore } from './settings.svelte';
import { OVERLAY_WIDGETS, type OverlayWidgetId } from '../lib/overlay-widgets';

const SPECS = new Map(OVERLAY_WIDGETS.map(w => [w.id, w]));

export function widgetPosition(id: OverlayWidgetId): { x: number; y: number } {
  return settingsStore.settings.widgetPositions?.[id]
      ?? SPECS.get(id)!.defaultPosition;
}

export function setWidgetPosition(id: OverlayWidgetId, x: number, y: number): void {
  settingsStore.set('widgetPositions', {
    ...settingsStore.settings.widgetPositions,
    [id]: { x, y },
  });
}
```

`widgetPosition` is reactive when consumed via `$derived(widgetPosition(id))`
because `settingsStore.settings` is `$state`.

### `DragGhost.svelte` — primitive

```svelte
<script lang="ts">
  interface Props {
    label: string;
    x: number; y: number;          // percent
    width: number; height: number; // px
    onmove:   (x: number, y: number) => void; // during drag (visual)
    oncommit: (x: number, y: number) => void; // on mouseup (persist)
  }
  let { label, x, y, width, height, onmove, oncommit }: Props = $props();

  let dragging = $state(false);
  let offX = 0, offY = 0;

  const clamp = (v: number, lo: number, hi: number) =>
    Math.min(Math.max(v, lo), hi);

  function onDown(e: MouseEvent) {
    e.preventDefault(); e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    offX = e.clientX - r.left; offY = e.clientY - r.top;
    dragging = true;
  }
  function onMove(e: MouseEvent) {
    if (!dragging) return;
    const w = window.innerWidth, h = window.innerHeight;
    const pxX = e.clientX - offX, pxY = e.clientY - offY;
    const maxX = 100 - (width / w) * 100;
    const maxY = 100 - (height / h) * 100;
    onmove(
      clamp((pxX / w) * 100, 0, Math.max(0, maxX)),
      clamp((pxY / h) * 100, 0, Math.max(0, maxY)),
    );
  }
  function onUp() {
    if (!dragging) return;
    dragging = false;
    oncommit(x, y);
  }
</script>

<svelte:window onmousemove={onMove} onmouseup={onUp} />

<div class="ghost" class:dragging
     style="top: {y}%; left: {x}%; width: {width}px; height: {height}px;"
     onmousedown={onDown} role="button" tabindex="-1"
     aria-label="Drag {label}">
  <span class="ghost-label">{label}</span>
</div>

<style>
  .ghost {
    position: absolute; box-sizing: border-box;
    border: 2px dashed var(--accent-primary, #6aa3ff);
    background: rgba(106, 163, 255, 0.15);
    border-radius: 4px;
    display: flex; align-items: center; justify-content: center;
    color: var(--text-primary, #e0e0e0);
    font-family: var(--font-mono, monospace); font-size: 13px;
    text-align: center; cursor: grab; user-select: none;
    pointer-events: auto;
  }
  .ghost:hover { background: rgba(106, 163, 255, 0.25); }
  .ghost.dragging { cursor: grabbing; background: rgba(106, 163, 255, 0.35); }
  .ghost-label { pointer-events: none; }
</style>
```

### `OverlayEditGrid.svelte` — rewritten

```svelte
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { OVERLAY_WIDGETS } from '../lib/overlay-widgets';
  import { widgetPosition, setWidgetPosition } from '../stores/widget-positions';
  import DragGhost from './DragGhost.svelte';

  // Snapshot taken on mount; mutated during drag for smooth visuals;
  // committed on mouseup so settings only see one write per drag.
  let pending = $state(
    Object.fromEntries(
      OVERLAY_WIDGETS.map(w => [w.id, { ...widgetPosition(w.id) }]),
    ) as Record<string, { x: number; y: number }>,
  );

  // User releases the edit chord mid-drag: OverlayWindow flips editActive
  // to false and unmounts us before DragGhost ever sees mouseup. Flush any
  // pending positions on teardown so the partial drag survives.
  onDestroy(() => {
    for (const w of OVERLAY_WIDGETS) {
      const p = pending[w.id];
      const stored = widgetPosition(w.id);
      if (p.x !== stored.x || p.y !== stored.y) {
        setWidgetPosition(w.id, p.x, p.y);
      }
    }
  });
</script>

<div class="edit-grid">
  {#each OVERLAY_WIDGETS as widget (widget.id)}
    <DragGhost
      label={widget.label}
      x={pending[widget.id].x}
      y={pending[widget.id].y}
      width={widget.ghostSize.width}
      height={widget.ghostSize.height}
      onmove={(x, y) => pending[widget.id] = { x, y }}
      oncommit={(x, y) => setWidgetPosition(widget.id, x, y)}
    />
  {/each}
</div>

<style>
  .edit-grid {
    position: fixed; inset: 0;
    pointer-events: none;          /* THE fix */
    z-index: 10000;
    background-image:
      linear-gradient(to right, rgba(180, 180, 255, 0.12) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(180, 180, 255, 0.12) 1px, transparent 1px),
      linear-gradient(to right, rgba(180, 180, 255, 0.22) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(180, 180, 255, 0.22) 1px, transparent 1px);
    background-size: 25px 25px, 25px 25px, 100px 100px, 100px 100px;
    background-color: rgba(0, 0, 0, 0.25);
  }
</style>
```

The grid takes no props. Old `x/y/onchange` interface goes away.

### `OverlayWindow.svelte` simplification

```svelte
listen<{ active: boolean }>('overlay-edit-mode', async (event) => {
  editActive = event.payload.active;
  await invoke('set_overlay_interactive', { active: editActive || historyVisible });
});
```

Removed: `pendingX/pendingY`, `notificationX/notificationY` derived state,
the `setNotificationPosition` call, the `x`/`y` props on `<NotificationStack>`,
the `editActive` prop on `<DpsMeter>`. `<OverlayEditGrid />` rendered with
no props.

## Refactor plan (per-component)

Single PR. Suggested commit ordering keeps each step buildable when
sequenced together; commits 1 and 2 do not compile in isolation, so they
should be reviewed together with commit 3.

### 1. Settings + migration

- `src-tauri/src/settings.rs`: add `WidgetPosition` and
  `widget_positions`. Remove `notification_x`, `notification_y`,
  `dps_meter.position`. Update defaults.
- `src-tauri/src/migrations/mod.rs` and
  `src-tauri/src/migrations/v1_24_widget_positions.rs`: as specified above.
- `src/stores/settings.svelte.ts`: add `WidgetPosition`,
  `widgetPositions`, drop the legacy fields and methods
  (`setNotificationPosition`, `setDpsMeterPosition`). No Rust commands
  to drop — both setters were TS-only; positions write via the existing
  `save_settings` command.

### 2. New module

- `src/lib/overlay-widgets.ts` (registry).
- `src/stores/widget-positions.svelte.ts` (helpers).
- `src/components/DragGhost.svelte`.
- Rewrite `src/components/OverlayEditGrid.svelte`.
- Export from `src/components/index.ts`.

### 3. Migrate `NotificationStack`

- Drop `x`, `y` props.
- Read `widgetPosition('notifications')` via `$derived`.
- Style with `top: {y}%; left: {x}%;`.
- In `OverlayWindow.svelte`: drop derived `notificationX/Y`,
  `pendingX/pendingY`, the `x/y` props on `<NotificationStack>`, the
  `setNotificationPosition` call. Simplify the `overlay-edit-mode`
  listener as shown above. Render `<OverlayEditGrid />` with no props.

### 4. Migrate `DpsMeter`

- Drop the `editActive` prop and all drag handlers
  (`onPanelMouseDown`, `onWindowMouseMove`, `onWindowMouseUp`,
  `dragOffsetX/Y`, `dragging`, `position` `$state`, the position
  `$effect`).
- Drop the `<svelte:window>` listener.
- Read `widgetPosition('dps-meter')` via `$derived`.
- Render with `style:top` / `style:left` in percent.
- CSS: drop `.edit-active` and `.dragging` rules. Drop
  `pointer-events: auto` from edit-active state. Drop `cursor: grab`
  / `grabbing`. Widget is now `pointer-events: none` always.
- In `OverlayWindow.svelte`: render `<DpsMeter />` without
  `editActive`.

### 5. Migrate `LootHistoryPanel`

- Read `widgetPosition('loot-history')` via `$derived`.
- Replace transform-centering CSS:
  ```css
  /* before */
  position: fixed;
  top: 50%; left: 50%;
  transform: translate(-50%, -50%);
  /* after */
  position: fixed;
  /* top/left supplied inline */
  ```
- Render `style:top="{pos.y}%" style:left="{pos.x}%"`.
- No new settings field, no per-panel drag code.

## Manual verification checklist

1. **Existing user (legacy `settings.json`)**: launch the upgraded app.
   Notifications appear at the same anchor as before. After any subsequent
   save, `settings.json` contains `widget_positions["notifications"]` and
   no longer contains `notificationX/Y`.
2. **Fresh install (`settings.json` deleted)**: launch. All three
   widgets appear at their registry-default positions.
3. **Drag notifications**: hold the edit chord. The grid appears, plus
   three ghost rectangles. Drag the notifications ghost. Release.
   Trigger a drop in-game. The notification appears at the new spot.
4. **Drag DPS meter (visible)**: enable DPS meter, hold the edit chord,
   drag its ghost, release. The real meter snaps to the new spot.
5. **Drag DPS meter (hidden)**: disable DPS meter, hold the edit chord,
   the DPS ghost is still visible and draggable. Drag, release. Re-enable
   the meter. It appears at the new position.
6. **Drag loot history (closed)**: do not press the loot-history hotkey.
   Hold the edit chord. The history ghost is visible at the registry
   default. Drag, release. Press the loot-history hotkey. The panel
   opens at the new position.
7. **Cross-window sync**: open the main window and the overlay. Drag a
   ghost in the overlay. The change persists across an app restart and
   is not clobbered by any settings save from the main window.
8. **Click-through restoration**: after releasing the edit chord (and
   with loot history closed), the overlay returns to click-through —
   clicks pass through to the game.
9. **Pointer events**: while edit chord is held, hovering the grid
   background between ghosts does not change the cursor and does not
   block underlying widgets visually. Only ghost rectangles respond to
   the mouse.
10. **Chord released mid-drag**: hold the edit chord, start dragging a
    ghost, release the chord while still holding the mouse button. The
    grid disappears. The widget keeps the position it was being dragged
    to (committed by the grid's `onDestroy` flush).

## Out of scope

- Snap-to-grid, alignment guides, multi-select drag.
- Resize handles. Adding `resizable: boolean` to the registry spec is
  feasible later but unneeded today.
- Animated snap when committing a new position.
- Per-resolution position profiles.
- Showing widget positions or providing reset-to-default in the General
  tab UI.
