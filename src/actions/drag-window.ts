import type { Action } from 'svelte/action';

export interface WindowPosition {
  x: number;
  y: number;
}

interface DragWindowOptions {
  target: () => HTMLElement | null;
  onMove: (position: WindowPosition) => void;
}

const clamp = (value: number, min: number, max: number): number =>
  Math.min(Math.max(value, min), max);

function isInteractiveTarget(target: EventTarget | null): boolean {
  return target instanceof Element && Boolean(target.closest('button, input, textarea, select, a'));
}

export const dragWindow: Action<HTMLElement, DragWindowOptions> = (handle, options) => {
  let currentOptions = options;
  let dragging = false;
  let offsetX = 0;
  let offsetY = 0;
  let width = 0;
  let height = 0;

  function moveTo(event: PointerEvent) {
    if (!dragging) return;

    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    if (viewportWidth === 0 || viewportHeight === 0) return;

    const left = clamp(event.clientX - offsetX, 0, Math.max(0, viewportWidth - width));
    const top = clamp(event.clientY - offsetY, 0, Math.max(0, viewportHeight - height));

    currentOptions.onMove({
      x: (left / viewportWidth) * 100,
      y: (top / viewportHeight) * 100,
    });
  }

  function stopDragging() {
    if (!dragging) return;
    dragging = false;
    window.removeEventListener('pointermove', moveTo, true);
    window.removeEventListener('pointerup', stopDragging, true);
    window.removeEventListener('pointercancel', stopDragging, true);
  }

  function startDragging(event: PointerEvent) {
    if (event.button !== 0 || isInteractiveTarget(event.target)) return;

    const target = currentOptions.target();
    if (!target) return;

    const rect = target.getBoundingClientRect();
    offsetX = event.clientX - rect.left;
    offsetY = event.clientY - rect.top;
    width = rect.width;
    height = rect.height;
    dragging = true;

    event.preventDefault();
    event.stopPropagation();
    window.addEventListener('pointermove', moveTo, true);
    window.addEventListener('pointerup', stopDragging, true);
    window.addEventListener('pointercancel', stopDragging, true);
  }

  handle.addEventListener('pointerdown', startDragging);

  return {
    update(options) {
      currentOptions = options;
    },
    destroy() {
      stopDragging();
      handle.removeEventListener('pointerdown', startDragging);
    },
  };
};
