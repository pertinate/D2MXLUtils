# Overlay widget repositioning — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one reusable module that handles drag/persistence/edit-mode for every overlay widget (notifications, DPS meter, loot history, plus future widgets). Fix the regression that prevented dragging DPS meter and loot history.

**Architecture:** A static widget registry (`OVERLAY_WIDGETS`) declares each repositionable widget. Positions live in a single centralized `widgetPositions` settings dictionary, in percent of overlay size. The redesigned `OverlayEditGrid` iterates the registry and renders one `DragGhost` per widget — ghosts are always present during edit mode regardless of whether the real widget is currently rendered. The grid background becomes pointer-events: none, fixing the bug where it was swallowing all clicks.

**Tech Stack:** Rust (Tauri v2 backend), Svelte 5 with runes, TypeScript, tauri-plugin-store.

**Spec:** `docs/superpowers/specs/2026-05-09-overlay-widget-repositioning-design.md`.

---

## File Structure

**Created:**
- `src-tauri/src/migrations/mod.rs` — orchestrator for settings migrations
- `src-tauri/src/migrations/v1_24_widget_positions.rs` — migrate `notificationX/Y` → `widget_positions["notifications"]`
- `src/lib/overlay-widgets.ts` — static registry (`OVERLAY_WIDGETS`)
- `src/stores/widget-positions.svelte.ts` — `widgetPosition()` / `setWidgetPosition()` helpers
- `src/components/DragGhost.svelte` — low-level drag primitive

**Modified:**
- `src-tauri/src/settings.rs` — add `WidgetPosition` + `widget_positions`; later remove `notification_x/y` and `dps_meter.position`; wire migration into `load_settings`
- `src-tauri/src/main.rs` — declare `mod migrations;`
- `src/stores/settings.svelte.ts` — add `WidgetPosition` + `widgetPositions`; later remove legacy fields and the `setNotificationPosition` / `setDpsMeterPosition` methods
- `src/stores/index.ts` — export new types
- `src/components/index.ts` — export `DragGhost`
- `src/components/OverlayEditGrid.svelte` — full rewrite (no props, iterates registry)
- `src/components/NotificationStack.svelte` — drop `x`/`y` props, read via helper
- `src/components/DpsMeter.svelte` — drop drag logic + `editActive` prop, read via helper
- `src/components/LootHistoryPanel.svelte` — switch from transform-centering to top/left percent, read via helper
- `src/views/OverlayWindow.svelte` — simplify edit-mode listener; drop `pendingX/Y`, drop `x`/`y` on `<NotificationStack>`, drop `editActive` on `<DpsMeter>`

---

## Task 1: Rust — settings types + migration module (TDD)

**Files:**
- Create: `src-tauri/src/migrations/mod.rs`
- Create: `src-tauri/src/migrations/v1_24_widget_positions.rs`
- Modify: `src-tauri/src/settings.rs` — add `WidgetPosition` and `widget_positions`; wire migration into `load_settings`. **Keep** `notification_x`, `notification_y`, and `dps_meter.position` for now (Task 6 removes them).
- Modify: `src-tauri/src/main.rs` — declare `mod migrations;`

The struct stays buildable for callers because we only ADD a field. Frontend keeps working because legacy fields are still serialized.

- [ ] **Step 1.1: Add `WidgetPosition` type and `widget_positions` field in `settings.rs`**

In `src-tauri/src/settings.rs`, after the existing imports:

```rust
use std::collections::HashMap;
```

After `DpsMeterPosition` (around line 34), add:

```rust
/// Position of an overlay widget, expressed as a percentage of the
/// overlay (0..=100 on each axis).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPosition {
    pub x: f64,
    pub y: f64,
}
```

In `AppSettings` (after `dps_meter: DpsMeterSettings,` around line 148), add:

```rust
    /// Centralized positions for repositionable overlay widgets, keyed by
    /// widget id (see `src/lib/overlay-widgets.ts`). Percent of overlay size.
    #[serde(default)]
    pub widget_positions: HashMap<String, WidgetPosition>,
```

In `impl Default for AppSettings`, add to the struct literal:

```rust
            widget_positions: HashMap::new(),
```

- [ ] **Step 1.2: Declare the migrations module in `main.rs`**

In `src-tauri/src/main.rs`, add to the module declarations (alphabetical placement near line 13–14):

```rust
mod migrations;
```

- [ ] **Step 1.3: Create `migrations/mod.rs`**

```rust
//! Settings migrations applied at load time.
//!
//! Adding a migration:
//!   1. Create `migrations/v<version>_<topic>.rs` with a single
//!      `pub fn apply(raw: &Value, s: &mut AppSettings) -> bool`.
//!   2. Add a `mod` declaration and a call below.
//!
//! Each migration must be idempotent (gate on the new field's
//! presence). After `migrate()` returns true, `settings.rs` re-saves
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

- [ ] **Step 1.4: Write the failing tests for the v1.24 migration**

Create `src-tauri/src/migrations/v1_24_widget_positions.rs` with **only** the test module (the `apply` function arrives in step 1.5):

```rust
//! v1.23 → v1.24: top-level `notificationX/Y` (percent) moved into
//! `widget_positions["notifications"]`. Done as part of the unified
//! overlay-widget-repositioning module — see
//! docs/superpowers/specs/2026-05-09-overlay-widget-repositioning-design.md.

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

pub fn apply(_raw: &Value, _s: &mut AppSettings) -> bool {
    false // unimplemented; tests should fail here
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_settings() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn migrates_when_legacy_keys_present() {
        let raw = json!({ "notificationX": 42.5, "notificationY": 17.0 });
        let mut s = fresh_settings();

        let changed = apply(&raw, &mut s);

        assert!(changed, "should report changed=true");
        assert_eq!(
            s.widget_positions.get("notifications"),
            Some(&WidgetPosition { x: 42.5, y: 17.0 }),
        );
    }

    #[test]
    fn idempotent_when_already_migrated() {
        let raw = json!({ "notificationX": 99.0, "notificationY": 99.0 });
        let mut s = fresh_settings();
        s.widget_positions
            .insert("notifications".into(), WidgetPosition { x: 5.0, y: 7.0 });

        let changed = apply(&raw, &mut s);

        assert!(!changed, "should report changed=false on second run");
        assert_eq!(
            s.widget_positions.get("notifications"),
            Some(&WidgetPosition { x: 5.0, y: 7.0 }),
            "must not overwrite an existing entry",
        );
    }

    #[test]
    fn skips_when_neither_legacy_key_present() {
        // Fresh install: no legacy keys, no widget_positions["notifications"].
        // The helper's spec default kicks in client-side, so we should NOT
        // pollute settings.json with a redundant entry.
        let raw = json!({});
        let mut s = fresh_settings();

        let changed = apply(&raw, &mut s);

        assert!(!changed);
        assert!(s.widget_positions.get("notifications").is_none());
    }

    #[test]
    fn partial_legacy_uses_default_for_missing_axis() {
        let raw = json!({ "notificationX": 50.0 });
        let mut s = fresh_settings();

        let changed = apply(&raw, &mut s);

        assert!(changed);
        assert_eq!(
            s.widget_positions.get("notifications"),
            Some(&WidgetPosition { x: 50.0, y: 1.0 }),
        );
    }
}
```

- [ ] **Step 1.5: Run the tests to verify they fail**

```bash
cd src-tauri && cargo test migrations::v1_24_widget_positions
```

Expected: 2 of 4 fail (`migrates_when_legacy_keys_present`, `partial_legacy_uses_default_for_missing_axis`). The other two pass trivially because the no-op `apply` happens to satisfy them — `idempotent_when_already_migrated` returns false correctly (no overwrite because no insert); `skips_when_neither_legacy_key_present` returns false correctly (no insert). Confirms the two real cases drive the implementation.

- [ ] **Step 1.6: Implement `apply` to make the tests pass**

Replace the placeholder `apply` in `src-tauri/src/migrations/v1_24_widget_positions.rs` with:

```rust
pub fn apply(raw: &Value, s: &mut AppSettings) -> bool {
    if s.widget_positions.contains_key("notifications") {
        return false;
    }
    let legacy: LegacyKeys = serde_json::from_value(raw.clone()).unwrap_or_default();
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

- [ ] **Step 1.7: Run the tests to verify they pass**

```bash
cd src-tauri && cargo test migrations::v1_24_widget_positions
```

Expected: 4 passed, 0 failed.

- [ ] **Step 1.8: Wire migration into `load_settings`**

In `src-tauri/src/settings.rs`, replace the body of `load_settings` (currently lines 263–281) with:

```rust
#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<AppSettings, String> {
    let store = app
        .store(SETTINGS_FILE)
        .map_err(|e| format!("Failed to open settings store: {}", e))?;

    let Some(raw) = store.get("settings") else {
        log_info("No settings found, using defaults");
        return Ok(AppSettings::default());
    };

    let mut settings: AppSettings = serde_json::from_value(raw.clone())
        .unwrap_or_else(|e| {
            log_error(&format!("Failed to parse settings, using defaults: {}", e));
            AppSettings::default()
        });

    if crate::migrations::migrate(&raw, &mut settings) {
        match serde_json::to_value(&settings) {
            Ok(value) => {
                store.set("settings", value);
                if let Err(e) = store.save() {
                    log_error(&format!("Failed to save migrated settings: {}", e));
                } else {
                    log_info("Settings migrated and re-saved");
                }
            }
            Err(e) => {
                log_error(&format!("Failed to serialize migrated settings: {}", e));
            }
        }
    }

    Ok(settings)
}
```

- [ ] **Step 1.9: Build and run the test suite**

```bash
cd src-tauri && cargo build
cd src-tauri && cargo test
```

Expected: build succeeds, all tests pass (existing tests untouched).

- [ ] **Step 1.10: Commit**

```bash
git add src-tauri/src/migrations src-tauri/src/main.rs src-tauri/src/settings.rs
git commit -m "feat(settings): add widget_positions field + v1.24 migration"
```

---

## Task 2: Frontend — registry, store helper, DragGhost component

**Files:**
- Create: `src/lib/overlay-widgets.ts`
- Create: `src/stores/widget-positions.svelte.ts`
- Create: `src/components/DragGhost.svelte`
- Modify: `src/stores/settings.svelte.ts` — add `WidgetPosition` interface and `widgetPositions` field to `AppSettings` and `DEFAULT_SETTINGS`. Keep legacy fields and methods for now.
- Modify: `src/stores/index.ts` — re-export `WidgetPosition`
- Modify: `src/components/index.ts` — export `DragGhost`

After this task: new infrastructure exists, no behavior change yet.

- [ ] **Step 2.1: Create the registry**

Create `src/lib/overlay-widgets.ts`:

```ts
/**
 * Static registry of every repositionable overlay widget.
 *
 * Adding a new widget:
 *   1. Add an entry to OVERLAY_WIDGETS below.
 *   2. In the widget's component, read its position via
 *      `widgetPosition(id)` from `src/stores/widget-positions.svelte.ts`.
 *   3. Style with `top: {y}%; left: {x}%;` (percent of overlay size).
 *
 * IDs are settings keys — never rename one after release.
 */

export interface OverlayWidgetSpec {
  /** Stable id used as the settings key. NEVER change after release. */
  id: string;
  /** Shown on the ghost label during edit mode. */
  label: string;
  /** Position used when the widget has no saved position yet. Percent. */
  defaultPosition: { x: number; y: number };
  /** Approximate rendered size in pixels — sizes the ghost and clamps drag. */
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

export type OverlayWidgetId = typeof OVERLAY_WIDGETS[number]['id'];
```

- [ ] **Step 2.2: Add `WidgetPosition` and `widgetPositions` to the settings store**

In `src/stores/settings.svelte.ts`, near the other interfaces (after `DpsMeterPosition` at line ~37), add:

```ts
export interface WidgetPosition {
  /** Percent of overlay width, 0..100 */
  x: number;
  /** Percent of overlay height, 0..100 */
  y: number;
}
```

In `AppSettings` interface, after `dpsMeter: DpsMeterSettings;` (around line 86), add:

```ts
  /** Centralized positions for repositionable overlay widgets, keyed by id.
   *  See `src/lib/overlay-widgets.ts`. Percent of overlay size. */
  widgetPositions: Record<string, WidgetPosition>;
```

In `DEFAULT_SETTINGS` (around line 158), add a trailing entry inside the object literal:

```ts
  widgetPositions: {},
```

In `src/stores/index.ts`, extend the existing first export line so it also exports `WidgetPosition`:

```ts
export { settingsStore, windowState, type AppSettings, type WindowState, type HotkeyConfig, type SoundSlot, type SoundSource, type DpsMeterSettings, type DpsMeterPosition, type WidgetPosition } from './settings.svelte';
```

- [ ] **Step 2.3: Create the position store helper**

Create `src/stores/widget-positions.svelte.ts`:

```ts
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
```

- [ ] **Step 2.4: Create the `DragGhost` component**

Create `src/components/DragGhost.svelte`:

```svelte
<script lang="ts">
  /**
   * Low-level drag primitive used by `OverlayEditGrid`.
   *
   * Coordinates are percentages (0..100) of the viewport.
   * `width` and `height` are pixels — used to size the ghost and to
   * clamp the drag so the ghost never escapes the visible area.
   */
  interface Props {
    label: string;
    x: number;
    y: number;
    width: number;
    height: number;
    /** Fires on every mousemove during drag (visual feedback). */
    onmove: (x: number, y: number) => void;
    /** Fires on mouseup (persistence). */
    oncommit: (x: number, y: number) => void;
  }

  let { label, x, y, width, height, onmove, oncommit }: Props = $props();

  let dragging = $state(false);
  let offX = 0;
  let offY = 0;

  const clamp = (v: number, lo: number, hi: number): number =>
    Math.min(Math.max(v, lo), hi);

  function onDown(e: MouseEvent): void {
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    offX = e.clientX - r.left;
    offY = e.clientY - r.top;
    dragging = true;
  }

  function onMove(e: MouseEvent): void {
    if (!dragging) return;
    const w = window.innerWidth;
    const h = window.innerHeight;
    if (w === 0 || h === 0) return;
    const pxX = e.clientX - offX;
    const pxY = e.clientY - offY;
    const maxX = 100 - (width / w) * 100;
    const maxY = 100 - (height / h) * 100;
    onmove(
      clamp((pxX / w) * 100, 0, Math.max(0, maxX)),
      clamp((pxY / h) * 100, 0, Math.max(0, maxY)),
    );
  }

  function onUp(): void {
    if (!dragging) return;
    dragging = false;
    oncommit(x, y);
  }
</script>

<svelte:window onmousemove={onMove} onmouseup={onUp} />

<div
  class="ghost"
  class:dragging
  style="top: {y}%; left: {x}%; width: {width}px; height: {height}px;"
  onmousedown={onDown}
  role="button"
  tabindex="-1"
  aria-label="Drag {label}"
>
  <span class="ghost-label">{label}</span>
</div>

<style>
  .ghost {
    position: absolute;
    box-sizing: border-box;
    border: 2px dashed var(--accent-primary, #6aa3ff);
    background: rgba(106, 163, 255, 0.15);
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-primary, #e0e0e0);
    font-family: var(--font-mono, monospace);
    font-size: 13px;
    text-align: center;
    cursor: grab;
    user-select: none;
    pointer-events: auto; /* opt back into pointer events; the grid disables them */
    transition: background 120ms ease;
  }

  .ghost:hover {
    background: rgba(106, 163, 255, 0.25);
  }

  .ghost.dragging {
    cursor: grabbing;
    background: rgba(106, 163, 255, 0.35);
  }

  .ghost-label {
    pointer-events: none;
  }
</style>
```

- [ ] **Step 2.5: Export `DragGhost` from the components barrel**

In `src/components/index.ts`, add a line in the "Notification components" group (anywhere before `OverlayEditGrid` is fine):

```ts
export { default as DragGhost } from './DragGhost.svelte';
```

- [ ] **Step 2.6: Build the frontend to verify no type errors**

```bash
pnpm tauri dev
```

Expected: app launches normally. `widget_positions` is empty in the settings store; new modules are present but unused. The DPS meter still works via its current pixel-based code; notifications still drag via the old grid.

Stop the dev server (Ctrl+C) once it boots cleanly.

- [ ] **Step 2.7: Commit**

```bash
git add src/lib/overlay-widgets.ts src/stores/widget-positions.svelte.ts src/stores/index.ts src/stores/settings.svelte.ts src/components/DragGhost.svelte src/components/index.ts
git commit -m "feat(overlay): add widget registry, position store, DragGhost primitive"
```

---

## Task 3: Migrate notifications to the new system (rewrite OverlayEditGrid)

**Files:**
- Modify: `src/components/OverlayEditGrid.svelte` — full rewrite
- Modify: `src/components/NotificationStack.svelte` — drop `x`/`y` props, read via helper
- Modify: `src/views/OverlayWindow.svelte` — drop `pendingX/pendingY`, drop `notificationX/notificationY` derived state, drop `setNotificationPosition` call, drop `x`/`y` props on `<NotificationStack>`, simplify the `overlay-edit-mode` listener, render `<OverlayEditGrid />` without props

After this task: holding the chord shows ghosts for ALL widgets (including the still-pixel-based DPS meter and the not-yet-draggable history panel — but DPS / history widgets themselves still don't read from `widgetPositions`, so dragging their ghosts has no visible effect yet). Notifications fully use the new path. The migration runs at app start, so existing users' notification anchor is preserved.

- [ ] **Step 3.1: Rewrite `OverlayEditGrid.svelte`**

Replace the entire contents of `src/components/OverlayEditGrid.svelte` with:

```svelte
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { OVERLAY_WIDGETS } from '../lib/overlay-widgets';
  import {
    widgetPosition,
    setWidgetPosition,
  } from '../stores/widget-positions';
  import DragGhost from './DragGhost.svelte';

  // Snapshot taken on mount; mutated during drag for smooth visuals;
  // committed on mouseup so settings only see one write per drag.
  let pending = $state(
    Object.fromEntries(
      OVERLAY_WIDGETS.map((w) => [w.id, { ...widgetPosition(w.id) }]),
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
      onmove={(x, y) => (pending[widget.id] = { x, y })}
      oncommit={(x, y) => setWidgetPosition(widget.id, x, y)}
    />
  {/each}
</div>

<style>
  .edit-grid {
    position: fixed;
    inset: 0;
    /* KEY FIX: was 'auto'. Only ghost children opt back into pointer events. */
    pointer-events: none;
    z-index: 10000;
    background-image:
      linear-gradient(to right, rgba(180, 180, 255, 0.12) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(180, 180, 255, 0.12) 1px, transparent 1px),
      linear-gradient(to right, rgba(180, 180, 255, 0.22) 1px, transparent 1px),
      linear-gradient(to bottom, rgba(180, 180, 255, 0.22) 1px, transparent 1px);
    background-size:
      25px 25px,
      25px 25px,
      100px 100px,
      100px 100px;
    background-color: rgba(0, 0, 0, 0.25);
  }
</style>
```

- [ ] **Step 3.2: Rewrite `NotificationStack.svelte` to read from the store**

Replace the entire contents of `src/components/NotificationStack.svelte` with:

```svelte
<script lang="ts">
  import Notification from './Notification.svelte';
  import { widgetPosition } from '../stores/widget-positions';

  type UniqueKind = 'tu' | 'su' | 'ssu' | 'sssu';

  interface NotificationFilter {
    color?: string | null;
    sound?: number | null;
    display_stats: boolean;
    matched_stat_lines?: number[] | null;
  }

  interface ItemDrop {
    unit_id: number;
    class: number;
    quality: string;
    name: string;
    base_name: string;
    stats: string;
    is_ethereal: boolean;
    is_identified: boolean;
    unique_kind?: UniqueKind | null;
    filter?: NotificationFilter | null;
    exiting?: boolean;
  }

  interface Props {
    items: ItemDrop[];
    maxVisible?: number;
    fontSize?: number;
    opacity?: number;
    compactName?: boolean;
  }

  let {
    items,
    maxVisible = 10,
    fontSize = 14,
    opacity = 0.9,
    compactName = false,
  }: Props = $props();

  let pos = $derived(widgetPosition('notifications'));
  const visibleItems = $derived(items.slice(0, maxVisible));
</script>

<div
  class="notification-stack"
  style="top: {pos.y}%; left: {pos.x}%;"
>
  {#each visibleItems as item (item.unit_id)}
    <Notification
      {item}
      exiting={item.exiting ?? false}
      {fontSize}
      {opacity}
      {compactName}
    />
  {/each}
</div>

<style>
  .notification-stack {
    position: fixed;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    pointer-events: none;
    z-index: 9999;
  }

  .notification-stack > :global(*) {
    pointer-events: auto;
  }
</style>
```

- [ ] **Step 3.3: Simplify `OverlayWindow.svelte`**

In `src/views/OverlayWindow.svelte`, make these specific edits:

Remove the derived position lines (around lines 42–43):
```ts
  let notificationX = $derived(settingsStore.settings.notificationX);
  let notificationY = $derived(settingsStore.settings.notificationY);
```

Remove the pending-position state lines (around lines 48–49):
```ts
  let pendingX = $state(0);
  let pendingY = $state(0);
```

Replace the `overlay-edit-mode` listener (current code around lines 118–138) with:

```ts
    listen<{ active: boolean }>('overlay-edit-mode', async (event) => {
      editActive = event.payload.active;
      try {
        await invoke('set_overlay_interactive', {
          active: editActive || historyVisible,
        });
      } catch (err) {
        console.error('[Overlay] set_overlay_interactive failed:', err);
      }
    }).then(u => unlisteners.push(u));
```

Update the `<NotificationStack>` element (around lines 183–191) — drop the `x` and `y` props:

```svelte
  <NotificationStack
    {items}
    maxVisible={10}
    fontSize={notificationFontSize}
    opacity={notificationOpacity}
    {compactName}
  />
```

Update the `<OverlayEditGrid>` element (around lines 193–199) — drop all props:

```svelte
  {#if editActive}
    <OverlayEditGrid />
  {/if}
```

- [ ] **Step 3.4: Build and run**

```bash
pnpm tauri dev
```

- [ ] **Step 3.5: Manual verification — notifications**

With Diablo II open and the app attached:

1. Hold the edit chord (Ctrl+Alt by default). The grid background appears with three dashed ghosts (notifications, dps-meter, loot-history). Cursor over the grid background does NOT show a "no entry" — only the ghost rectangles respond.
2. Drag the **Drop notifications** ghost to a new position. Release. The grid stays visible; the chord is still held.
3. Release the chord. Trigger a drop in-game. The notification appears at the new position.
4. Open `settings.json` via General tab → "Open folder" button. Confirm `widgetPositions.notifications` reflects the dragged position. Legacy `notificationX/Y` still present (will be removed in Task 6).

If the migration ran (existing user with old `notificationX/Y`), the position should be preserved across the upgrade.

- [ ] **Step 3.6: Commit**

```bash
git add src/components/OverlayEditGrid.svelte src/components/NotificationStack.svelte src/views/OverlayWindow.svelte
git commit -m "feat(overlay): migrate notifications to centralized widget positions"
```

---

## Task 4: Migrate DPS meter to the new system

**Files:**
- Modify: `src/components/DpsMeter.svelte` — drop drag handlers, drop `editActive` prop, drop local position state, read via helper, switch to percent positioning
- Modify: `src/views/OverlayWindow.svelte` — drop `editActive` prop on `<DpsMeter>`

After this task: DPS meter is fully draggable through the new system. Its legacy `dps_meter.position` field still exists in settings but is unused.

- [ ] **Step 4.1: Rewrite `DpsMeter.svelte`**

Replace the entire contents of `src/components/DpsMeter.svelte` with:

```svelte
<script lang="ts">
  import { dpsMeterStore } from '../stores';
  import { widgetPosition } from '../stores/widget-positions';
  import { formatDps } from '../lib/format-dps';

  let pos = $derived(widgetPosition('dps-meter'));

  let snap = $derived(dpsMeterStore.state);
  let dpsStr = $derived(snap.inSession ? formatDps(snap.dps) : '—');
  let kpmStr = $derived(snap.inSession ? snap.kpm.toFixed(1) : '—');
  let peakStr = $derived(snap.inSession ? formatDps(snap.peak) : '—');
  let totalStr = $derived(snap.inSession ? formatDps(snap.total) : '—');
  let killsStr = $derived(snap.inSession ? snap.kills.toString() : '—');
</script>

<div
  class="dps-meter"
  class:in-session={snap.inSession}
  style:left="{pos.x}%"
  style:top="{pos.y}%"
>
  <div class="row"><span class="label">DPS</span><span class="value">{dpsStr}</span></div>
  <div class="row"><span class="label">Kills/min</span><span class="value">{kpmStr}</span></div>
  <div class="row"><span class="label">Peak</span><span class="value">{peakStr}</span></div>
  <div class="row"><span class="label">Total</span><span class="value">{totalStr}</span></div>
  <div class="row"><span class="label">Kills</span><span class="value">{killsStr}</span></div>
</div>

<style>
  .dps-meter {
    position: absolute;
    background: rgba(0, 0, 0, 0.55);
    color: #e8e8e8;
    padding: 6px 10px;
    border-radius: 4px;
    font-family: var(--font-mono, monospace);
    font-size: 12px;
    line-height: 1.35;
    opacity: 0.55;
    user-select: none;
    pointer-events: none; /* drag is owned by the ghost in OverlayEditGrid */
    transition: opacity 200ms ease;
  }

  .dps-meter.in-session {
    opacity: 0.95;
  }

  .row {
    display: grid;
    grid-template-columns: 5.5em 5em;
    gap: 6px;
  }

  .label {
    color: #aaa;
  }

  .value {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
```

Note: the `settingsStore` import is gone (no longer needed); `dpsMeterStore` import stays.

- [ ] **Step 4.2: Drop the `editActive` prop on `<DpsMeter>` in `OverlayWindow.svelte`**

Replace the `<DpsMeter editActive={editActive} />` element (around lines 206–208) with:

```svelte
  {#if dpsMeterEnabled}
    <DpsMeter />
  {/if}
```

- [ ] **Step 4.3: Build and run**

```bash
pnpm tauri dev
```

- [ ] **Step 4.4: Manual verification — DPS meter (visible)**

1. Enable the DPS meter (toggle in General tab).
2. Hold the edit chord. The DPS meter ghost appears (and remains visible even when the meter itself is "in session" / out of session).
3. Drag the DPS meter ghost to a new spot. Release. The real DPS meter snaps to the ghost's position.
4. Release the chord. Confirm the DPS meter stays at the new position. `settings.json` (via General tab → "Open folder") shows `widgetPositions["dps-meter"]` updated.

- [ ] **Step 4.5: Manual verification — DPS meter (hidden)**

1. Disable the DPS meter (toggle off).
2. Hold the edit chord. The DPS meter ghost is still visible at the saved position.
3. Drag the ghost to a new spot. Release. Release the chord.
4. Re-enable the DPS meter. It appears at the position the ghost was dragged to.

- [ ] **Step 4.6: Commit**

```bash
git add src/components/DpsMeter.svelte src/views/OverlayWindow.svelte
git commit -m "refactor(dps-meter): drop bespoke drag, use centralized repositioning"
```

---

## Task 5: Migrate loot history to the new system

**Files:**
- Modify: `src/components/LootHistoryPanel.svelte` — read via helper, switch CSS from `transform: translate(-50%, -50%)` centering to `top/left` percent

After this task: loot history fully draggable through the unified system.

- [ ] **Step 5.1: Update `LootHistoryPanel.svelte`**

In `src/components/LootHistoryPanel.svelte`, add an import near the top of the `<script>` block (after the `lootHistoryStore` import):

```ts
  import { widgetPosition } from '../stores/widget-positions';
```

Add a derived position inside the `<script>`:

```ts
  let pos = $derived(widgetPosition('loot-history'));
```

Update the root element to bind position inline. Replace:

```svelte
<div class="loot-history-panel" role="dialog" aria-label="Loot history">
```

with:

```svelte
<div
  class="loot-history-panel"
  role="dialog"
  aria-label="Loot history"
  style:top="{pos.y}%"
  style:left="{pos.x}%"
>
```

Update the `.loot-history-panel` CSS rule. Replace:

```css
  .loot-history-panel {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    max-width: min(700px, 60vw);
    width: 100%;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: rgba(0, 0, 0, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: var(--radius-md, 8px);
    color: #f0f0f0;
    pointer-events: auto;
    font-family: var(--font-mono, monospace);
    font-size: 13px;
  }
```

with:

```css
  .loot-history-panel {
    position: fixed;
    /* top/left supplied inline via style:top / style:left */
    max-width: min(700px, 60vw);
    width: 100%;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: rgba(0, 0, 0, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: var(--radius-md, 8px);
    color: #f0f0f0;
    pointer-events: auto;
    font-family: var(--font-mono, monospace);
    font-size: 13px;
  }
```

- [ ] **Step 5.2: Build and run**

```bash
pnpm tauri dev
```

- [ ] **Step 5.3: Manual verification — loot history**

1. Without opening the loot history (no hotkey press), hold the edit chord. The loot history ghost is visible at its registry default position.
2. Drag the ghost to a different spot. Release. Release the chord.
3. Press the loot history hotkey (default `N`). The panel opens at the dragged position.
4. Press `N` again to close. Re-open. Position is preserved.
5. `settings.json` (via General tab → "Open folder") shows `widgetPositions["loot-history"]` updated.

- [ ] **Step 5.4: Commit**

```bash
git add src/components/LootHistoryPanel.svelte
git commit -m "feat(loot-history): make panel repositionable via overlay edit mode"
```

---

## Task 6: Remove legacy fields and methods

**Files:**
- Modify: `src-tauri/src/settings.rs` — remove `notification_x`, `notification_y`, the `position` field of `DpsMeterSettings`, related defaults
- Modify: `src/stores/settings.svelte.ts` — remove same fields from TS, remove `setNotificationPosition` and `setDpsMeterPosition` methods, remove `DpsMeterPosition` type if unused
- Modify: `src/stores/index.ts` — drop `DpsMeterPosition` re-export

After this task: no dead code referring to the old fields. Existing users' settings.json still has `notificationX/Y` and `dpsMeter.position` until the next save, after which serde drops them silently (no `deny_unknown_fields`, so no error during the in-between state).

- [ ] **Step 6.1: Remove legacy fields from Rust struct**

In `src-tauri/src/settings.rs`:

Remove these fields from `AppSettings` (lines ~100–106):
```rust
    /// Notification position X offset from edge (percentage 0-100)
    #[serde(default = "default_notification_x")]
    pub notification_x: f32,

    /// Notification position Y offset from edge (percentage 0-100)
    #[serde(default = "default_notification_y")]
    pub notification_y: f32,
```

Remove the `position` field from `DpsMeterSettings` (lines ~21–22):
```rust
    #[serde(default)]
    pub position: Option<DpsMeterPosition>,
```

Remove the `DpsMeterPosition` struct entirely (lines ~29–34):
```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsMeterPosition {
    pub x: i32,
    pub y: i32,
}
```

Remove the `default_notification_x` and `default_notification_y` functions (lines ~187–193).

In `impl Default for AppSettings`, remove these two lines:
```rust
            notification_x: default_notification_x(),
            notification_y: default_notification_y(),
```

- [ ] **Step 6.2: Build and run Rust tests**

```bash
cd src-tauri && cargo build
cd src-tauri && cargo test
```

Expected: build succeeds, all tests pass (the migration tests already use only `WidgetPosition` and `AppSettings::default()`).

- [ ] **Step 6.3: Remove legacy fields from TS settings**

In `src/stores/settings.svelte.ts`:

Remove the `DpsMeterPosition` interface (around lines 34–37):
```ts
export interface DpsMeterPosition {
  x: number;
  y: number;
}
```

In `DpsMeterSettings` interface, remove the `position` field:
```ts
  position: DpsMeterPosition | null;
```

In `AppSettings` interface, remove the `notificationX` and `notificationY` fields (around lines 62–65).

In `DEFAULT_SETTINGS`, remove the corresponding lines (around lines 141–142):
```ts
  notificationX: 1.0,
  notificationY: 1.0,
```

In the `DEFAULT_SETTINGS.dpsMeter` literal, remove the `position: null,` line.

Remove the `setNotificationPosition` method (around lines 405–407):
```ts
  setNotificationPosition(x: number, y: number): void {
    this.update({ notificationX: x, notificationY: y });
  }
```

Remove the `setDpsMeterPosition` method (around lines 413–418):
```ts
  setDpsMeterPosition(x: number, y: number): void {
    this.set('dpsMeter', {
      ...this._settings.dpsMeter,
      position: { x, y },
    });
  }
```

- [ ] **Step 6.4: Remove `DpsMeterPosition` from the stores barrel export**

In `src/stores/index.ts`, remove the `DpsMeterPosition` from the first export line. The line should now be:

```ts
export { settingsStore, windowState, type AppSettings, type WindowState, type HotkeyConfig, type SoundSlot, type SoundSource, type DpsMeterSettings, type WidgetPosition } from './settings.svelte';
```

- [ ] **Step 6.5: Build and verify nothing references the removed names**

```bash
pnpm tauri dev
```

If anything still references `notificationX`, `notificationY`, `DpsMeterPosition`, `setNotificationPosition`, `setDpsMeterPosition`, or `dpsMeter.position`, the TypeScript compiler will report it. Fix each reference (usually a leftover read or import).

Stop the dev server once it boots cleanly.

- [ ] **Step 6.6: Manual smoke check**

1. Launch app once (this re-saves settings, dropping the legacy keys from JSON).
2. Open `settings.json` — confirm `notificationX`, `notificationY`, and `dpsMeter.position` are gone, and `widgetPositions` is the only place positions live.
3. Trigger a drop, drag the notifications ghost, drag the DPS meter ghost, drag the loot history ghost — everything still works as in Tasks 3-5.

- [ ] **Step 6.7: Commit**

```bash
git add src-tauri/src/settings.rs src/stores/settings.svelte.ts src/stores/index.ts
git commit -m "refactor(settings): drop legacy notification/dps-meter position fields"
```

---

## Task 7: Final end-to-end verification

This is the spec's checklist run in full against a built artefact. No code changes.

- [ ] **Step 7.1: Existing-user path (legacy `settings.json` migration)**

1. Before starting this task, on a clean checkout of `master` (pre-Task 1), launch the app and confirm `settings.json` contains `notificationX` and `notificationY` (drag the ghost to a non-default spot to make it visible).
2. Switch back to the implementation branch and build/run.
3. Confirm the notifications appear at the same anchor as before the upgrade.
4. After any subsequent settings save, `settings.json` contains `widgetPositions["notifications"]` matching the pre-upgrade values, and no longer contains `notificationX`/`notificationY`.

- [ ] **Step 7.2: Fresh-install path**

1. Stop the app. Delete `settings.json` (or rename it).
2. Launch the app. All three widgets render at their registry-default positions (notifications top-right, DPS meter top-right, loot history near center).
3. `settings.json` after first save does NOT contain a `widgetPositions["notifications"]` entry (the migration's "skip when neither legacy key present" branch took effect). The helper's spec defaults are used at read time.

- [ ] **Step 7.3: Drag every widget**

For each widget (notifications, DPS meter, loot history):
- With the widget visible, hold the chord, drag, release. Real widget snaps to the new spot.
- With the widget hidden (notifications between drops; DPS toggled off; loot history closed), hold the chord, drag the ghost, release. Make the widget visible again — it appears at the new spot.

- [ ] **Step 7.4: Cross-window sync**

1. Open the main window AND the overlay simultaneously.
2. Drag a ghost in the overlay; release.
3. Restart the app. Position is preserved.
4. Repeat with the main window open and modifying any unrelated setting (e.g. toggle verbose logging) right after the drag — confirm the drag is not clobbered (this exercises the dirty-keys merge logic).

- [ ] **Step 7.5: Click-through restoration**

After the chord is released and loot history is closed, the overlay is click-through (clicks on the game pass through). With loot history open, the overlay accepts clicks (so the user can interact with the panel).

- [ ] **Step 7.6: Pointer-events sanity (the original bug)**

While the chord is held, hover the grid background between ghosts. Cursor stays default (no `grab`), and underlying widgets are not visually blocked. Only ghost rectangles respond to clicks.

- [ ] **Step 7.7: Chord released mid-drag (the edge case)**

1. Hold the chord, start dragging the DPS meter ghost.
2. Without releasing the mouse, release the chord. Grid disappears.
3. Release the mouse anywhere.
4. Re-open the overlay. The DPS meter sits at the position the ghost reached when the chord was released (committed by `OverlayEditGrid.onDestroy`).

- [ ] **Step 7.8: Final commit (only if any fixes were needed)**

If steps 7.1–7.7 surfaced bugs, fix and commit:

```bash
git add <changed files>
git commit -m "fix(overlay): <specific issue>"
```

If everything passed, no commit needed — the feature is done.
