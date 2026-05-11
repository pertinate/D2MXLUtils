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
  let lastX = x;
  let lastY = y;

  const clamp = (v: number, lo: number, hi: number): number => Math.min(Math.max(v, lo), hi);

  function onDown(e: MouseEvent): void {
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    offX = e.clientX - r.left;
    offY = e.clientY - r.top;
    dragging = true;
    lastX = x;
    lastY = y;
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
    const nx = clamp((pxX / w) * 100, 0, Math.max(0, maxX));
    const ny = clamp((pxY / h) * 100, 0, Math.max(0, maxY));
    lastX = nx;
    lastY = ny;
    onmove(nx, ny);
  }

  function onUp(): void {
    if (!dragging) return;
    dragging = false;
    oncommit(lastX, lastY);
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
