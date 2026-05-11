<script lang="ts">
  import { onDestroy } from 'svelte';
  import { OVERLAY_WIDGETS } from '../lib/overlay-widgets';
  import {
    widgetPosition,
    setWidgetPosition,
  } from '../stores/widget-positions.svelte';
  import DragGhost from './DragGhost.svelte';

  // Snapshot taken on mount; mutated during drag for smooth visuals;
  // committed on mouseup so settings only see one write per drag.
  let pending = $state(
    Object.fromEntries(
      OVERLAY_WIDGETS.map((w) => [w.id, { ...widgetPosition(w.id) }]),
    ) as Record<string, { x: number; y: number }>,
  );

  // User releases the edit chord mid-drag: the edit window unmounts us before
  // DragGhost ever sees mouseup. Flush pending positions so the partial drag
  // survives.
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
