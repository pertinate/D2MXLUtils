# DPS Meter — Design Spec (event-driven via inline hook)

> **Pivot note**: This spec was rewritten on 2026-05-08 after RE revealed
> that polling HP via `ReadProcessMemory` has fundamental accuracy
> problems on MXL (one-shot zero-credit, reconcile false-positives,
> normalization scale issues). The new architecture uses an inline hook
> on `D2Common::STATLIST_SetUnitStat` to capture damage events at the
> moment they're applied. See [`docs/dps-meter-reverse-engineering.md`](../../dps-meter-reverse-engineering.md)
> for the full RE record. This pivot also unlocks **multiplayer support**,
> which the user requires (most MXL players are on private realms).

## Overview

Add a live DPS overlay to the existing in-game overlay window. Use case: testing a build's throughput on a reference fat monster (boss) or in a reference area, then comparing the same metric across builds. All damage sources count — player, mercenary, summons, item procs — there is no per-source attribution.

The meter shows five numbers in a draggable panel:

- **DPS** — rolling 5-second average damage per second
- **Kills/min** — rolling 5-second kill rate, scaled to per-minute (× 12)
- **Peak** — max DPS sample observed since last session reset
- **Total** — total damage accumulated since last session reset
- **Kills** — total monster kills since last session reset

A session starts implicitly when the player enters an area; reset is triggered by area change (now in v1 — Level chain was RE'd 2026-05-08) or via a manual reset hotkey. Visibility of the panel is toggled by another hotkey.

A kill is detected unambiguously: the hook fires `Ord10887(pUnit, statId=6, value=0, ...)` when the server sets a monster's HP to zero. No reconcile heuristic, no false-positives from off-screen monsters — every kill is a real explicit event from the server.

## Goals

- Live overlay readout while playing, no main-window tab work
- Counts every damage event regardless of source (no attribution, by design)
- **Works in both single-player and multiplayer** — hook target is in D2Common.dll which is client-side in both modes
- **Absolute damage numbers** (matching real monster HP scale) via MonStats `wMaxHP[difficulty]` lookup
- Manual reset for re-runs in the same area without leaving
- Negligible game-side performance impact (hook fires only on stat writes; early-out on non-HP/non-monster writes is ~5 instructions)

## Non-goals

- Per-source / per-skill / per-damage-type breakdown
- Cross-area session aggregation
- DPS history graphs, logs, exports
- Configurable rolling-window size (hardcoded 5 s in v1)
- Drag-to-resize panel
- Showing the meter outside an active game (out-of-game state simply hides the readout)
- ~~Auto-reset on area change~~ — RE'd 2026-05-08, IS in v1 scope (`offsets.rs::level_chain`)
- DoT damage capture at application time (poison/burn writes to `STAT_HPREGEN`, our hook only catches stat 6 — DoT damage is captured but only when regen-tick spreads it into stat 6, with one game-tick delay)

---

## Section 1: RE foundation (DONE)

All offsets and the hook target are verified live against MXL on a multiplayer realm. See [`docs/dps-meter-reverse-engineering.md`](../../dps-meter-reverse-engineering.md) for full record. Summary:

- **Hook target**: `D2Common.dll + 0x3A740` (`Ord10887` = `STATLIST_SetUnitStat`)
- **Sig**: `__stdcall(pUnit, statId, value, layer)`, `ret 0010`
- **Prologue**: `8B 44 24 0C 53` (5-byte overwriteable for inline `jmp`)
- **Universal sink**: 3+ cases in D2Client packet dispatcher all converge here; same path in SP (loopback packet) and MP (network packet)

### The 32768 protocol quirk (key insight)

Server transmits monster HP as **a 0-127 byte percentage**. Client scales locally:
```
hp_pct = packet & 0x7F                ; 7-bit percentage
if (hp_pct > 1) hp_pct++              ; off-by-one normalization
engine_hp = hp_pct << 8               ; max = 128 * 256 = 32768
```

Therefore engine HP delta is proportional to **real-HP percentage**:
```
real_damage = (engine_delta / 32768) × MonStats.wMaxHP[difficulty]
```

This is the formula our scanner-side accumulator uses. MonStats records were already RE'd in earlier work (`offsets.rs::monstats_txt`) — `wMaxHP[Normal/NM/Hell]` at `+0xB0/+0xB2/+0xB4`.

### MP compatibility verification

Confirmed during RE: D2Game.dll IS loaded in the MP client process, but our hook path never goes through it. Call stack on damage:
```
[mov [eax+04],ecx]   ← Ord10261+0x79 (D2Common, leaf writer)
↑ Ord10887+0x1F      (D2Common, hook target)
↑ D2Client.dll+0x4E195   (giant packet dispatcher case A)
↑ D2Client.dll+0xD2F00   (caller — top-level handler not yet RE'd)
```

No D2Game frames. Same call chain in SP and MP — only the source of the packet differs.

---

## Section 2: Architecture & data flow

```
┌─ Game thread (D2 process, hooked) ─────────────────────────┐
│                                                             │
│  Server packet → D2Client packet handler                   │
│       ↓                                                     │
│  D2Client.dll+0x4BE70 (dispatcher) → call thunk → ...      │
│       ↓                                                     │
│  Ord10887 entry hooked ──→ our trampoline                  │
│                                ├─ filter: statId, type,    │
│                                │           is_spawn        │
│                                ├─ read old_HP inline       │
│                                ├─ if delta < 0:            │
│                                │   push event to ring      │
│                                └─ resume Ord10887 prologue │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                  │
                  │ shared memory ring buffer (single-producer,
                  │ single-consumer, lock-free atomic head/tail)
                  ↓
┌─ D2MXLUtils scanner thread (Rust) ─────────────────────────┐
│                                                             │
│  every ~16-50 ms (per scanner tick):                       │
│    drain ring buffer                                        │
│    for each event { ts, unit_id, delta_raw, max_hp }:       │
│        damage = (delta_raw / 32768) × max_hp                │
│        rolling_window.push(ts, damage)                     │
│        session_total += damage                             │
│        session_peak = max(session_peak, current_dps)        │
│    emit("dps-update", { dps, peak, total, in_session })    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                  │
                  ↓ Tauri event
┌─ Overlay window (Svelte) ──────────────────────────────────┐
│  DpsMeter component subscribes to dps-update               │
│  Renders DPS / Peak / Total in draggable panel             │
└─────────────────────────────────────────────────────────────┘
```

### File layout

**New (Rust):**
- `src-tauri/src/dps_hook/mod.rs` — public surface; init/shutdown
- `src-tauri/src/dps_hook/trampoline.rs` — assembled trampoline bytes (machine code generated programmatically), install/uninstall
- `src-tauri/src/dps_hook/ring.rs` — ring-buffer reader (consumer side, in our process — actual buffer is in D2's address space)
- `src-tauri/src/dps_meter.rs` — pure-Rust accumulator (rolling window, peak, total, formatter)

**New (Svelte):**
- `src/components/DpsMeter.svelte` — overlay panel
- `src/stores/dps-meter.svelte.ts` — frontend state subscribed from `dps-update`
- `src/lib/format-dps.ts` — number formatter (3 sig fig + unit suffix; `12.3k`, `1.20M`)

**Modified:**
- `src-tauri/src/offsets.rs` — add `stat_list` module + new D2Common RVAs (`STATLIST_SET_UNIT_STAT`, `STATLIST_SET_STAT`)
- `src-tauri/src/notifier.rs` — call `dps_hook.drain_into(dps_meter)` per scanner tick; emit `dps-update` event
- `src-tauri/src/scanner_state.rs` — embed `dps_hook: Arc<DpsHook>` and `dps_meter: Arc<RwLock<DpsMeter>>`
- `src-tauri/src/main.rs` — instantiate state on attach, install hook on first attach, uninstall on detach; register Tauri commands and hotkeys
- `src-tauri/src/hotkeys.rs` — add `ToggleDpsMeter`, `ResetDpsSession`
- `src-tauri/src/settings.rs` — add `dps_meter` field
- `src/views/OverlayWindow.svelte` — render `<DpsMeter />` conditionally
- `src/views/GeneralTab.svelte` — add "DPS Meter" section

### Events & Tauri commands

**Events (backend → frontend):**
- `dps-update`: `{ dps: f32, peak: f32, total: u64, in_session: bool }` — every ~100 ms while `enabled && attached_to_game`. When `in_session = false`, panel renders placeholders.

**Commands (frontend → backend):**
- `set_dps_meter_enabled(enabled: bool)` — toggles overlay visibility, persists
- `set_dps_meter_position(x: i32, y: i32)` — debounced save after drag
- `reset_dps_session()` — manual reset

---

## Section 3: Inline hook design

### Hook target

`Ord10887` at `D2Common.dll + 0x3A740`, prologue:
```asm
+0x00  8B 44 24 0C    mov eax, [esp+0xC]    ; 4 bytes
+0x04  53             push ebx              ; 1 byte
+0x05  ...                                  ; rest of prologue
```

**Hook installation:** overwrite bytes `[+0x00 .. +0x04]` (5 bytes) with `E9 rel32 = jmp our_trampoline`. The relative offset is `(trampoline_addr - (Ord10887 + 5))`. The original 5 bytes are preserved in the trampoline for execution after our handler.

### Trampoline structure

```asm
trampoline:
    ; SAVE STATE — we entered via jmp, no return-addr was pushed
    pushfd                              ; save flags
    pushad                              ; save all 8 GP regs (32 bytes)
    
    ; ────────────────────────────────────────────────────────────────
    ; READ ARGS
    ; pushad+pushfd added 0x24 bytes, args are at +0x24..+0x30
    ; ────────────────────────────────────────────────────────────────
    mov eax, [esp+0x28]                 ; pUnit       (originally [esp+4])
    mov ecx, [esp+0x2C]                 ; statId
    mov edx, [esp+0x30]                 ; new value
    ; layer at [esp+0x34] — usually 0, ignored
    
    ; ────────────────────────────────────────────────────────────────
    ; FILTER — early out cheaply
    ; ────────────────────────────────────────────────────────────────
    cmp ecx, 6                          ; STAT_HITPOINTS?
    jne .restore
    
    test eax, eax                       ; pUnit valid?
    jz .restore
    cmp dword [eax+0x00], 1             ; UnitAny.dwType == UNIT_MONSTER?
    jne .restore
    
    mov esi, [eax+0x14]                 ; pUnitData
    test esi, esi
    jz .restore
    mov esi, [esi]                      ; pMonStatsRecord = pUnitData[0]
    test esi, esi
    jz .restore
    
    movzx ebx, byte [esi+0x4C]          ; isSpawn
    test ebx, ebx
    jnz .restore                        ; isSpawn != 0 → summon, skip
    
    ; ────────────────────────────────────────────────────────────────
    ; READ OLD HP — walk pUnit.pStats inline (faster than calling GetUnitStat)
    ; ────────────────────────────────────────────────────────────────
    mov ebx, [eax+0x5C]                 ; pStatListEx
    test ebx, ebx
    jz .restore
    mov edi, [ebx+0x24]                 ; pStat (for monsters: == ebx + 0x80)
    movzx ecx, word [ebx+0x28]          ; wStatCount1 (used count)
    test ecx, ecx
    jz .restore
    
    xor ebx, ebx                        ; old_hp = 0 (default if not found)
.find_hp:
    cmp word [edi + ebx*8 + 0x02], 6    ; nStat == HITPOINTS?
    je .got_hp
    inc ebx
    cmp ebx, ecx
    jb .find_hp
    jmp .restore                        ; not found → bail
.got_hp:
    mov ebx, [edi + ebx*8 + 0x04]       ; old_hp = dwValue
    
    ; ────────────────────────────────────────────────────────────────
    ; CHECK new < old (damage, not regen / set max)
    ; ────────────────────────────────────────────────────────────────
    cmp edx, ebx
    jae .restore                        ; new >= old → not damage, ignore
    
    sub ebx, edx                        ; ebx = delta_raw = old - new
    
    ; Encode kill flag into high bit of delta_raw (delta values fit in
    ; ~16 bits, so bit 31 is unused). new_value (edx) was 0 ⇒ kill.
    test edx, edx
    jnz .not_kill
    or  ebx, 0x80000000                 ; set kill flag
.not_kill:
    
    ; ────────────────────────────────────────────────────────────────
    ; PUSH EVENT to ring buffer
    ; Layout per slot: { u32 ts_ms, u32 unit_id, u32 delta_raw_with_kill_flag, u32 max_hp }
    ;                  delta_raw uses bit 31 as is_kill; bits 0-30 = damage delta
    ; Buffer header: { u32 head, u32 tail, u32 capacity, u32 _pad }
    ;                followed by capacity slots (16 bytes each)
    ; ────────────────────────────────────────────────────────────────
    push ebx                            ; delta_raw + kill flag
    push esi                            ; pMonStatsRecord (ring writer reads max_hp from it)
    push eax                            ; pUnit (ring writer reads unit_id)
    call our_ring_push_handler          ; cdecl, writes one event, returns nothing
    add esp, 0xC                        ; cleanup
    
.restore:
    popad
    popfd
    
    ; ────────────────────────────────────────────────────────────────
    ; EXECUTE SAVED 5 BYTES (the original prologue we overwrote)
    ; ────────────────────────────────────────────────────────────────
    mov eax, [esp+0xC]
    push ebx
    
    ; JMP back to Ord10887 + 5
    jmp Ord10887_plus_5                 ; absolute via E9 rel32
```

### Ring-buffer push handler

A small subroutine (also in injected memory) that:
1. Reads current difficulty from D2Client static (small RE TODO; see Section 6)
2. Reads `monstats_max_hp = u16 at (pMonStatsRecord + 0xB0 + difficulty * 2)`
3. Reads `unit_id = u32 at (pUnit + 0x0C)`
4. Reads `now_ms` via `KERNEL32.GetTickCount`
5. Computes ring slot: `slot = head & (capacity - 1)`
6. Writes `{now_ms, unit_id, delta_raw, monstats_max_hp}` to slot
7. Atomically increments `head` (`lock xadd`)

Single-producer (only the game thread runs this) → no producer-side locking needed beyond the head atomic.

### Memory allocation strategy

- `VirtualAllocEx(d2_process, NULL, 0x4000, MEM_COMMIT, PAGE_EXECUTE_READWRITE)` — single 16 KB page split into:
  - First 256 bytes: ring buffer header + slots[16] = 16 events × 16 bytes = 256B... actually slots[1024] = 16 KB - header. Let's go with 1024 slots = 16384 bytes for slots + 16 bytes header. Round up to one 16 KB page.
  - Actually: simpler to allocate 2 pages (8 KB header+helpers, 16 KB ring) — one for trampoline + push-handler code, one for ring data.
- Reuse existing `injection.rs::INJECT_BASE` infrastructure for the IAT-like patching pattern (we already write to D2 memory there). For the trampoline + helper code, allocate fresh `VirtualAllocEx` region — don't intermix with the GetStringById injection at `0xCDE00`.

### Hook installation/uninstallation

- `dps_hook::install(process_handle, d2common_base)`:
  1. `VirtualAllocEx` for trampoline + helpers + ring buffer
  2. Write trampoline bytes (with patched `jmp` offsets to `Ord10887+5` and to `our_ring_push_handler`)
  3. Write ring buffer header (`head=0, tail=0, capacity=1024`)
  4. **Atomically** patch `Ord10887[+0..+4]` with `E9 rel32` pointing to trampoline
     (use `WriteProcessMemory` + `FlushInstructionCache`; original 5 bytes already saved in trampoline)
  5. Save state for uninstall (process handle, RVA, original 5 bytes, allocated region addr)

- `dps_hook::uninstall(state)`:
  1. Restore original 5 bytes at `Ord10887[+0..+4]`
  2. Wait briefly (50 ms?) to let in-flight trampoline executions finish
  3. `VirtualFreeEx` the allocated region

Race: between unpatching and freeing the region, a thread may still be inside the trampoline. The 50 ms wait is a heuristic. Defensive alternative: allocate the trampoline region with `MEM_RESERVE` only after unpatching, so freeing it later is safe even with in-flight executions. Defer until smoke test reveals if this is an actual problem.

---

## Section 4: Scanner-side accumulator

### Pure-Rust `DpsMeter`

```rust
struct Event {
    ts_ms: u64,
    damage: u32,
    is_kill: bool,
}

pub struct DpsMeter {
    events: VecDeque<Event>,
    session_total: u64,
    session_peak: f32,
    session_kills: u32,
}

impl DpsMeter {
    /// `delta_raw_with_flag`: bit 31 = is_kill flag (set by trampoline when
    /// new HP value was 0), bits 0-30 = actual delta.
    pub fn ingest(&mut self, ts_ms: u64, delta_raw_with_flag: u32, monstats_max_hp: u16) {
        let is_kill = (delta_raw_with_flag & 0x8000_0000) != 0;
        let delta_raw = delta_raw_with_flag & 0x7FFF_FFFF;
        let damage = ((delta_raw as u64 * monstats_max_hp as u64) / 32768) as u32;
        if damage == 0 && !is_kill { return; } // skip noise
        self.events.push_back(Event { ts_ms, damage, is_kill });
        self.session_total += damage as u64;
        if is_kill { self.session_kills += 1; }
    }

    pub fn snapshot(&mut self, now_ms: u64) -> DpsSnapshot {
        let cutoff = now_ms.saturating_sub(WINDOW_MS);
        while let Some(e) = self.events.front() {
            if e.ts_ms < cutoff { self.events.pop_front(); } else { break; }
        }
        let window_dmg: u64 = self.events.iter().map(|e| e.damage as u64).sum();
        let window_kills: u32 = self.events.iter().filter(|e| e.is_kill).count() as u32;
        let dps = window_dmg as f32 / WINDOW_SECONDS;
        let kpm = (window_kills as f32) * (60.0 / WINDOW_SECONDS); // × 12 for 5s window
        if dps > self.session_peak { self.session_peak = dps; }
        DpsSnapshot {
            dps,
            kpm,
            peak: self.session_peak,
            total: self.session_total,
            kills: self.session_kills,
            in_session: !self.events.is_empty() || self.session_total > 0,
        }
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.session_total = 0;
        self.session_peak = 0.0;
        self.session_kills = 0;
    }
}
```

Constants:
```rust
pub const WINDOW_SECONDS: f32 = 5.0;
const WINDOW_MS: u64 = 5_000;
```

Snapshot DTO:
```rust
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsSnapshot {
    pub dps: f32,        // damage per second (rolling)
    pub kpm: f32,        // kills per minute (rolling)
    pub peak: f32,       // max DPS sample seen this session
    pub total: u64,      // cumulative damage this session
    pub kills: u32,      // cumulative kill count this session
    pub in_session: bool,
}
```

### Drain loop (in `notifier::tick`)

```rust
pub fn tick(...) {
    // ... existing item-scan work ...

    if let Some(hook) = scanner_state.dps_hook.as_ref() {
        let mut meter = scanner_state.dps_meter.write();
        let now_ms = monotonic_now_ms();
        for event in hook.drain() {
            meter.ingest(event.ts_ms, event.delta_raw, event.max_hp);
        }
        let snapshot = meter.snapshot(now_ms);
        app_handle.emit("dps-update", &snapshot)?;
    }
}
```

### Reset

Triggered by:
1. **Manual hotkey** (`ResetDpsSession`) — Tauri command sets a flag; next tick performs the reset
2. **Player exits game** — scanner detaches; meter resets, emits final `dps-update` with zeros
3. **Area change** — automatic. Scanner reads `dwLevelNo` via `offsets.rs::level_chain` chain on each tick; on change, calls `meter.reset()`. RE'd 2026-05-08.

Reset operation: clear events, set `session_total = 0`, `session_peak = 0`, `session_kills = 0`.

---

## Section 5: UI — overlay component & hotkeys

(Carries over from previous spec with minor wording updates.)

### Layout

```
┌──────────────────┐
│ DPS:      12.3k  │
│ Kills/min: 47.2  │
│ Peak:     28.1k  │
│ Total:    1.20M  │
│ Kills:    237    │
└──────────────────┘
```

- Five rows, fixed grid (label column left-aligned, value column right-aligned, monospace numerics)
- Kills/min uses 1 decimal (`12.5`, `47.2`); Kills uses integer (`237`); DPS/Peak/Total use 3 sig fig + unit suffix
- Semi-transparent dark background (`rgba(0,0,0,0.5)`), 6 px padding, 4 px border-radius
- Number formatting: 3 sig fig + unit suffix. Below 1000 → integer. Thresholds at `1e3`, `1e6`, `1e9`. Examples: `0`, `42`, `999`, `1.00k`, `12.3k`, `123k`, `1.00M`, `12.3M`, `1.00B`.
- When `in_session = false`: render `—` in all three value cells, dim panel to 50% opacity

### Drag-to-move

- Whole panel surface = drag handle
- Local-state translate during drag, debounced `set_dps_meter_position` on `pointerup`
- Bounds clamping: keep ≥ 20 px on-screen
- Default position: top-right (`x = window_width - panel_width - 20`, `y = 20`)

### Hotkeys

Two new actions in `hotkeys.rs`:
- `ToggleDpsMeter` — flips `dps_meter.enabled`
- `ResetDpsSession` — calls `reset_dps_session`

Both default to unbound. Bind via General-tab UX (mirror existing hotkey rows).

### General tab — "DPS Meter" section

```
DPS Meter
─────────
[ ] Show overlay              ← bound to settings.dps_meter.enabled
Toggle overlay hotkey:    [unbound]   [Bind]
Reset session hotkey:     [unbound]   [Bind]

ℹ Note: requires single-player or multiplayer; works across both modes.
   DoT damage (poison/burn) is captured with one-tick delay.
```

(The note text is informational, mirroring how other settings explain caveats.)

---

## Section 6: Settings & persistence

(Same as previous spec.)

```rust
#[serde(default)]
pub dps_meter: DpsMeterSettings,

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DpsMeterSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub position: Option<DpsMeterPosition>,
    #[serde(default)]
    pub hotkey_toggle: Option<String>,
    #[serde(default)]
    pub hotkey_reset: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsMeterPosition {
    pub x: i32,
    pub y: i32,
}
```

Persistence via existing `tauri-plugin-store` pipeline. Migration: `#[serde(default)]` makes the section optional.

`window_seconds` hardcoded to 5.0 in v1.

---

## Section 7: Difficulty index RE (DONE 2026-05-09)

`D2Client.dll + 0x11C390` (`u32`: `0 = Normal`, `1 = Nightmare`, `2 = Hell`).

Resolved by 2-pass cross-run diff (`docs/ce-scripts/find-difficulty.lua`):
scanned `D2Client.dll +0..+0x180000` for `u32 == 0` on Normal, restarted on
Nightmare and filtered to `u32 == 1` — narrowed 12 200+ candidates to one.

Constant: `offsets.rs::d2client::DIFFICULTY = 0x11C390`.

---

## Section 8: Risks & mitigations

| Risk | Mitigation |
|---|---|
| Trampoline crashes the game (instability of inline hooks) | Smoke-test with one-second-soak after install; uninstall on Ctrl+C / panic via signal handler. Keep the patched 5 bytes saved so uninstall is reliable. |
| Race between uninstall and in-flight trampoline executions | 50 ms wait after unpatch before VirtualFree. If problematic, use MEM_RESERVE-only freeing or leak the region. |
| MXL ships an update that changes RVAs | RE work has to be redone. Document RVAs in `docs/dps-meter-reverse-engineering.md` so re-verification is fast (CE scripts already capture the methodology). |
| DoT damage capture delayed | Document in tooltip. Future v2 can hook `STAT_HPREGEN` writes too, but adds noise (regen/poison/burn all share that stat). |
| Difficulty index RE blocks v1 | Hardcode Hell. Document as a known limitation. RE in <1h once unblocked. |
| Hook misses one-shot kills | Hook fires on Ord10887 call, which fires before stat write. One-shot kill = single call with `value=0`. **Capture is reliable.** |
| Multiple Ord10887 calls per damage event (cascading stat updates) | In trampoline filter, deduplicate by `(unit_id, ts) where ts < previous_ts + 16ms`. Defer until smoke test reveals if this is an actual problem. |
| Player damage attributed (we want only monster damage) | Filter `pUnit.dwType == MONSTER` in trampoline. Verified. |
| Summon damage attributed | Filter `pUnit.MonStats.is_spawn == 0`. Verified. |

## Section 9: Out of scope

- Per-source / per-skill / per-damage-type breakdown
- Cross-area session aggregation
- DPS history graphs, logs, exports
- Configurable rolling-window size
- Multiple comparison sessions side-by-side
- Drag-to-resize panel
- ~~Auto-reset on area change~~ — RE done 2026-05-08, IS in v1
- DoT capture at application time (deferred to v2 — needs HPREGEN-stat hook)
- Per-source breakdown (player / merc / summons) — explicitly rejected by use case
