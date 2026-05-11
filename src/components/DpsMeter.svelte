<script lang="ts">
  import { dpsMeterStore } from '../stores';
  import { widgetPosition } from '../stores/widget-positions.svelte';
  import { formatDps } from '../lib/format-dps';

  let pos = $derived(widgetPosition('dps-meter'));

  let snap = $derived(dpsMeterStore.state);
  let dpsStr = $derived(formatDps(snap.dps));
  let kpmStr = $derived(snap.kpm.toFixed(1));
  let peakStr = $derived(formatDps(snap.peak));
  let totalStr = $derived(formatDps(snap.total));
  let killsStr = $derived(snap.kills.toString());
</script>

<div class="dps-meter" class:in-session={snap.inSession} style:left="{pos.x}%" style:top="{pos.y}%">
  <div class="row">
    <span class="label">DPS</span><span class="value">{dpsStr}</span>
  </div>
  <div class="row">
    <span class="label">Kills/min</span><span class="value">{kpmStr}</span>
  </div>
  <div class="row">
    <span class="label">Peak</span><span class="value">{peakStr}</span>
  </div>
  <div class="row">
    <span class="label">Total</span><span class="value">{totalStr}</span>
  </div>
  <div class="row">
    <span class="label">Kills</span><span class="value">{killsStr}</span>
  </div>
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
