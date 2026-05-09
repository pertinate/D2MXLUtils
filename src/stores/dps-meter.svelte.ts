import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** Wire format from `DpsSnapshot` (Rust → camelCase). */
export interface DpsSnapshot {
  dps: number;
  kpm: number;
  peak: number;
  total: number;
  kills: number;
  inSession: boolean;
}

const EMPTY: DpsSnapshot = {
  dps: 0,
  kpm: 0,
  peak: 0,
  total: 0,
  kills: 0,
  inSession: false,
};

class DpsMeterStore {
  state = $state<DpsSnapshot>({ ...EMPTY });
  #unlisten: UnlistenFn | null = null;
  #initialized = false;

  async initialize(): Promise<void> {
    if (this.#initialized) return;
    this.#initialized = true;
    try {
      this.#unlisten = await listen<DpsSnapshot>('dps-update', (event) => {
        this.state = event.payload;
      });
    } catch (err) {
      console.error('[DpsMeter] failed to subscribe to dps-update:', err);
    }
  }

  destroy(): void {
    if (this.#unlisten) {
      this.#unlisten();
      this.#unlisten = null;
    }
    this.#initialized = false;
  }
}

export const dpsMeterStore = new DpsMeterStore();
