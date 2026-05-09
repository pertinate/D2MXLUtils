<script lang="ts">
  import { dpsMeterStore, settingsStore } from '../stores';
  import { formatDps } from '../lib/format-dps';

  interface Props {
    /** True while the overlay-edit chord is held; panel becomes draggable. */
    editActive: boolean;
  }

  let { editActive }: Props = $props();

  let position = $state(
    settingsStore.settings.dpsMeter?.position ?? { x: 20, y: 20 },
  );

  // Adopt external position changes (e.g. from settings window) unless
  // the user is mid-drag.
  $effect(() => {
    const persisted = settingsStore.settings.dpsMeter?.position;
    if (persisted && (persisted.x !== position.x || persisted.y !== position.y)) {
      if (!dragging) {
        position = persisted;
      }
    }
  });

  let dragging = $state(false);
  let dragOffsetX = 0;
  let dragOffsetY = 0;

  function onPanelMouseDown(e: MouseEvent) {
    if (!editActive) return;
    e.preventDefault();
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dragOffsetX = e.clientX - rect.left;
    dragOffsetY = e.clientY - rect.top;
    dragging = true;
  }

  function onWindowMouseMove(e: MouseEvent) {
    if (!dragging) return;
    const w = window.innerWidth;
    const h = window.innerHeight;
    const next = {
      x: Math.max(0, Math.min(w - 80, e.clientX - dragOffsetX)),
      y: Math.max(0, Math.min(h - 40, e.clientY - dragOffsetY)),
    };
    position = next;
  }

  function onWindowMouseUp() {
    if (!dragging) return;
    dragging = false;
    settingsStore.setDpsMeterPosition(position.x, position.y);
  }

  let snap = $derived(dpsMeterStore.state);
  let dpsStr = $derived(snap.inSession ? formatDps(snap.dps) : '—');
  let kpmStr = $derived(snap.inSession ? snap.kpm.toFixed(1) : '—');
  let peakStr = $derived(snap.inSession ? formatDps(snap.peak) : '—');
  let totalStr = $derived(snap.inSession ? formatDps(snap.total) : '—');
  let killsStr = $derived(snap.inSession ? snap.kills.toString() : '—');
</script>

<svelte:window onmousemove={onWindowMouseMove} onmouseup={onWindowMouseUp} />

<div
  class="dps-meter"
  class:in-session={snap.inSession}
  class:edit-active={editActive}
  class:dragging
  style:left="{position.x}px"
  style:top="{position.y}px"
  onmousedown={onPanelMouseDown}
  role="presentation"
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
    pointer-events: none;
    transition: opacity 200ms ease;
  }

  .dps-meter.in-session {
    opacity: 0.95;
  }

  .dps-meter.edit-active {
    pointer-events: auto;
    cursor: grab;
    outline: 1px dashed var(--accent-primary, #6aa3ff);
    background: rgba(0, 0, 0, 0.65);
  }

  .dps-meter.dragging {
    cursor: grabbing;
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
