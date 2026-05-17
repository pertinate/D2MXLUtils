<script lang="ts">
  import { onMount } from 'svelte';
  import { clickOutside } from '../actions/click-outside';
  import { lootHistoryStore, type LootHistoryEntry } from '../stores';
  import { widgetPosition } from '../stores/widget-positions.svelte';

  let { onClose } = $props<{ onClose: () => void }>();

  let pos = $derived(widgetPosition('loot-history'));

  let scrollContainer: HTMLDivElement | null = $state(null);
  let stickToBottom = $state(true);

  // Palette mirrors Notification.svelte's nameColor cascade exactly:
  // explicit rule color → quality color → muted fallback.
  const notifyColors: Record<string, string> = {
    white: 'var(--notify-white)',
    red: 'var(--notify-red)',
    lime: 'var(--notify-lime)',
    blue: 'var(--notify-blue)',
    gold: 'var(--notify-gold)',
    grey: 'var(--notify-grey)',
    black: 'var(--notify-black)',
    pink: 'var(--notify-pink)',
    orange: 'var(--notify-orange)',
    yellow: 'var(--notify-yellow)',
    green: 'var(--notify-green)',
    purple: 'var(--notify-purple)',
  };

  const qualityColors: Record<string, string> = {
    Unique: 'var(--quality-unique)',
    Set: 'var(--quality-set)',
    Rare: 'var(--quality-rare)',
    Magic: 'var(--quality-magic)',
    Crafted: 'var(--quality-crafted)',
    Honorific: 'var(--quality-crafted)',
    Superior: 'var(--quality-superior)',
    Inferior: 'var(--quality-normal)',
    Normal: 'var(--quality-normal)',
  };

  function formatTime(ms: number): string {
    const d = new Date(ms);
    const hh = d.getHours().toString().padStart(2, '0');
    const mm = d.getMinutes().toString().padStart(2, '0');
    const ss = d.getSeconds().toString().padStart(2, '0');
    return `${hh}:${mm}:${ss}`;
  }

  function pickupIcon(state: LootHistoryEntry['pickup']): string {
    switch (state) {
      case 'picked_up':
        return '✓';
      case 'lost':
        return '⊘';
      case 'pending':
        return '⏳';
    }
  }

  function pickupClass(state: LootHistoryEntry['pickup']): string {
    return `pickup pickup-${state}`;
  }

  function nameColor(entry: LootHistoryEntry): string {
    // Final fallback is hardcoded — `--text-muted` flips to a dark value
    // under the light theme and would be invisible on our dark panel.
    return (
      (entry.color ? notifyColors[entry.color] : undefined) ??
      qualityColors[entry.quality] ??
      '#bdbdbd'
    );
  }

  function onScroll() {
    if (!scrollContainer) return;
    const el = scrollContainer;
    stickToBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 50;
  }

  // Auto-scroll to bottom only when the user is already near the bottom.
  $effect(() => {
    void lootHistoryStore.entries.length;
    if (stickToBottom && scrollContainer) {
      queueMicrotask(() => {
        if (scrollContainer) {
          scrollContainer.scrollTop = scrollContainer.scrollHeight;
        }
      });
    }
  });

  onMount(() => {
    void lootHistoryStore.initialize();
  });
</script>

<div
  class="loot-history-panel"
  use:clickOutside={onClose}
  role="dialog"
  aria-label="Loot history"
  style:top="{pos.y}%"
  style:left="{pos.x}%"
>
  <header>
    <h2>Loot History</h2>
    <div class="header-actions">
      <button
        type="button"
        class="clear-btn"
        onclick={() => lootHistoryStore.clear()}
        aria-label="Clear history">Clear</button
      >
      <button type="button" class="close" onclick={onClose} aria-label="Close">×</button>
    </div>
  </header>
  <div class="list" bind:this={scrollContainer} onscroll={onScroll}>
    {#each lootHistoryStore.entries as entry (entry.seed !== 0 ? `s:${entry.seed}` : `u:${entry.unit_id}`)}
      <div class="row">
        <span class="time">[{formatTime(entry.timestamp_ms)}]</span>
        <span class={pickupClass(entry.pickup)}>{pickupIcon(entry.pickup)}</span>
        <span class="name" style:color={nameColor(entry)}>{entry.name}</span>
      </div>
    {/each}
    {#if lootHistoryStore.entries.length === 0}
      <div class="empty">No drops in this session yet.</div>
    {/if}
  </div>
</div>

<style>
  .loot-history-panel {
    position: fixed;
    /* top/left supplied inline via style:top / style:left */
    width: min(560px, calc(100vw - 32px));
    max-height: min(520px, 70vh);
    display: flex;
    flex-direction: column;
    background: linear-gradient(180deg, rgba(12, 10, 8, 0.96), rgba(0, 0, 0, 0.92));
    border: 1px solid rgba(199, 179, 119, 0.5);
    box-shadow:
      0 12px 40px rgba(0, 0, 0, 0.75),
      inset 0 0 18px rgba(199, 179, 119, 0.08);
    color: #f3f0df;
    pointer-events: auto;
    font-family: var(--font-mono, Consolas, monospace);
    font-size: 13px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid rgba(199, 179, 119, 0.28);
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .header-actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .clear-btn {
    padding: 4px 8px;
    color: rgba(243, 240, 223, 0.78);
    background: rgba(0, 0, 0, 0.28);
    border: 1px solid rgba(199, 179, 119, 0.32);
    cursor: pointer;
    font: inherit;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .clear-btn:hover,
  .clear-btn:focus-visible {
    border-color: rgba(199, 179, 119, 0.55);
    background: rgba(255, 255, 255, 0.06);
    outline: none;
  }

  .close {
    color: #f3f0df;
    background: transparent;
    border: 0;
    cursor: pointer;
    font-size: 22px;
    line-height: 1;
    padding: 0 2px;
  }

  .close:hover,
  .close:focus-visible {
    opacity: 0.75;
    outline: none;
  }

  .list {
    overflow-y: auto;
    padding: 10px 8px;
    display: flex;
    flex-direction: column;
    gap: 0;
  }

  .row {
    display: grid;
    grid-template-columns: auto auto 1fr;
    align-items: baseline;
    gap: 8px;
    padding: 3px 8px;
    border: 1px solid transparent;
  }

  .time {
    color: rgba(243, 240, 223, 0.62);
    font-size: 12px;
    white-space: nowrap;
  }

  .pickup {
    width: 1em;
    text-align: center;
  }

  :global(.pickup-picked_up) {
    color: #5cd66a;
  }

  :global(.pickup-lost) {
    color: rgba(243, 240, 223, 0.42);
  }

  :global(.pickup-pending) {
    color: #f0b400;
  }

  .name {
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 14px;
    font-weight: 700;
  }

  .empty {
    padding: 20px 8px;
    text-align: center;
    color: rgba(243, 240, 223, 0.62);
    font-size: 12px;
  }
</style>
