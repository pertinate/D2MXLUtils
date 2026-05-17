import type { Action } from 'svelte/action';

type ClickOutsideHandler = (event: PointerEvent) => void;

export const clickOutside: Action<HTMLElement, ClickOutsideHandler> = (node, handler) => {
  let currentHandler = handler;

  function handlePointerDown(event: PointerEvent) {
    const target = event.target;
    if (target instanceof Node && !node.contains(target)) {
      currentHandler(event);
    }
  }

  window.addEventListener('pointerdown', handlePointerDown, true);

  return {
    update(handler) {
      currentHandler = handler;
    },
    destroy() {
      window.removeEventListener('pointerdown', handlePointerDown, true);
    },
  };
};
