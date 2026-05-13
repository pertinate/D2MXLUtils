<script lang="ts">
  import Notification from './Notification.svelte';
  import { widgetPosition } from '../stores/widget-positions.svelte';

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
    showOnlyMatchedStats?: boolean;
  }

  let {
    items,
    maxVisible = 10,
    fontSize = 14,
    opacity = 0.9,
    compactName = false,
    showOnlyMatchedStats = false,
  }: Props = $props();

  let pos = $derived(widgetPosition('notifications'));
  const visibleItems = $derived(items.slice(0, maxVisible));
</script>

<div class="notification-stack" style="top: {pos.y}%; left: {pos.x}%;">
  {#each visibleItems as item (item.unit_id)}
    <Notification
      {item}
      exiting={item.exiting ?? false}
      {fontSize}
      {opacity}
      {compactName}
      {showOnlyMatchedStats}
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
