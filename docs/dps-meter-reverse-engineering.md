# DPS Meter — Reverse Engineering & Status Report

> **Status: RE COMPLETE.** All offsets and architectural knowledge needed for
> the inline-hook implementation are identified and verified live against
> MXL on a multiplayer realm. **Ready to start implementation in a fresh
> session.** This document is the source-of-truth handoff — it intentionally
> over-documents incidental findings (D2Client/D2Common ordinals, struct
> layouts, calling conventions) so the next session can resume without
> re-running CE.

## References

- **Spec**: [`docs/superpowers/specs/2026-05-07-dps-meter-design.md`](superpowers/specs/2026-05-07-dps-meter-design.md) (event-driven, post-pivot)
- **Plan**: [`docs/superpowers/plans/2026-05-07-dps-meter.md`](superpowers/plans/2026-05-07-dps-meter.md) (event-driven, post-pivot)
- **CE scripts**: [`docs/ce-scripts/`](ce-scripts/) (all RE artifacts)
- **Verified offsets**: `src-tauri/src/offsets.rs` — `data_tables`, `monstats_txt` modules already committed

---

## TL;DR

**Hook target:** `STATLIST_SetUnitStat` at `D2Common.dll + 0x3A740` (exported as `Ord10887` per CE labels).
- `__stdcall(pUnit, statId, value, layer)`, 4 args, `ret 0010`
- 5-byte hot-patchable prologue: `8B 44 24 0C 53` (= `mov eax,[esp+0xC]; push ebx`) — perfect for inline `jmp` hook

**Why this point**: it's the **universal client-side sink** for all stat writes. Damage in MXL — whether SP or MP — flows through here:
- **SP**: local server (D2Game) computes damage → loopback packet → D2Client packet handler (`D2Client.dll+0x4BE70` giant switch) → `Ord10887` → `Ord10261` writer
- **MP**: remote server computes damage → real packet → same D2Client handler → same `Ord10887` → same writer

**🎉 Major insight — the 32768 normalization is a PROTOCOL QUIRK, not a feature of MXL design.**

Server transmits HP as a **0-127 byte percentage**. Client scales it locally:
```
hp_pct = packet_byte & 0x7F          ; 7-bit percent
if hp_pct > 1: hp_pct++              ; off-by-one adjustment
engine_hp = hp_pct << 8              ; max = 128 * 256 = 32768
```

This is why **all MXL monsters show stat 7 max == 32768 raw == 128 displayed** regardless of class/area/difficulty.

**🟢 PLAN B FORMULA NOW WORKS — with absolute numbers.**

Since engine HP IS proportional to real-HP percentage, and we have `wMaxHP[difficulty]` from MonStats records:
```
damage_abs = (delta_raw / 32768) × MonStats.wMaxHP[difficulty]
```

Treehead Hell `wMaxHP=600`, monster takes 28% damage → engine_delta = 0.28 × 32768 ≈ 9170, absolute_damage = 9170 / 32768 × 600 ≈ **168 HP** in real units.

Plan B was rejected earlier because polling missed events (one-shots, reconcile). **Hooking eliminates the polling problems**: every stat write fires the hook, no missed deltas, no kill-credit guesswork. Plan B's math + hook delivery = winning combo.

---

## Architecture (final)

```
┌─────────────────────────────────────────────────────────────────┐
│                        D2 client process                         │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  D2Client.dll                                             │   │
│  │                                                           │   │
│  │  (top-of-stack handler — not yet RE'd for damage path)   │   │
│  │       ↓                                                   │   │
│  │  +0x4BE70 (giant switch on packet type)                  │   │
│  │   case A: HP%-update packet      → +0x4E190 thunk-call   │   │
│  │   case B: HP-update from [ebx+14] → similar              │   │
│  │   case C: kill (set HP=0)         → similar              │   │
│  │       ↓                                                   │   │
│  │  +0xC40C  (thunk: jmp [+0xCE5FC])  ─────┐                │   │
│  └──────────────────────────────────────────┼─────────────────┘   │
│                                              ↓                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  D2Common.dll                                             │   │
│  │                                                           │   │
│  │   ★ Ord10887 (+0x3A740) STATLIST_SetUnitStat   ◄── HOOK  │   │
│  │       reads pUnit.pStats, calls...                       │   │
│  │       ↓                                                   │   │
│  │     Ord10261 (+0x3A280) STATLIST_SetStat                 │   │
│  │       finds stat in array, computes delta, calls...      │   │
│  │       ↓                                                   │   │
│  │     +0x79  mov [eax+04], ecx       ← actual HP write     │   │
│  │       ↓                                                   │   │
│  │     callbacks (Ord10379+0x2B0, Ord10871+0x30 if player)  │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Our injected trampoline (intercepts Ord10887 entry)     │   │
│  │  → reads pUnit/statId/value/layer                        │   │
│  │  → filters: statId==6, monster, isSpawn==0, value<old    │   │
│  │  → reads old HP (StatList walk inline)                   │   │
│  │  → push event {ts, unit_id, delta_raw, max_hp} to ring   │   │
│  │  → executes saved 5 bytes, jmps back to Ord10887+5       │   │
│  └─────────────────────┬────────────────────────────────────┘   │
│                        │ shared memory ring buffer (lock-free)  │
│                        ↓                                          │
└────────────────────────┼─────────────────────────────────────────┘
                         │
┌────────────────────────┼─────────────────────────────────────────┐
│                        ↓        D2MXLUtils Rust scanner          │
│  per scanner tick:                                                │
│  ├─ drain ring buffer                                             │
│  ├─ for each event: damage = (delta/32768) × max_hp               │
│  ├─ rolling-window accumulator (5 s)                              │
│  └─ emit `dps-update` event to overlay                            │
└──────────────────────────────────────────────────────────────────┘
```

---

## Verified offsets and addresses

**Notation:** all RVAs are relative to the named module's base address. Bases drift per launch; CE shows current bases at the top of every script's output.

### Mode-of-play context (for reproduction)

- **Game**: Median XL (Sigma) on a multiplayer realm — verified MP-compatibility
- **D2Client.dll base** during RE session: `0x6FAB0000`
- **D2Common.dll base** during RE session: `0x6FD50000`
- **D2Game.dll base** during RE session: `0x6FC20000` (loaded but NOT used by hook path — confirmed bypass)
- **Test character**: lvl 134 Sorceress (class 2), summoner build, on Hell
- **Test reference boss**: Treehead WoodFist in Dark Wood (`class_id = 2881`, `pUnit = 0x049BDA00` in one session)

### D2Common.dll RVAs

#### Already in `offsets.rs` (committed)

| Symbol | RVA | Notes |
|---|---|---|
| `SGPT_DATA_TABLES` | `0x99E1C` | Pointer to `D2DataTablesStrc` |
| `MONSTATS_TXT_PTR` | `+0xA78` (in DataTables struct) | Heap pointer to MonStats records |
| `MONSTATS_TXT_COUNT` | `+0xA80` | `nRecordCount = 3899` |
| `monstats_txt::RECORD_SIZE` | `0x1A8` | |
| `monstats_txt::W_ID` | `+0x00` (u16) | == record index |
| `monstats_txt::IS_SPAWN` | `+0x4C` (u8) | 0=wild, 1=spawnable |
| `monstats_txt::MIN_HP_*` | `+0xAA, +0xAC, +0xAE` (u16) | Normal/NM/Hell |
| `monstats_txt::MAX_HP_*` | `+0xB0, +0xB2, +0xB4` (u16) | Normal/NM/Hell — DPS scale |

#### To add (NEW for hook implementation)

| Symbol | RVA | Notes |
|---|---|---|
| `STATLIST_SET_UNIT_STAT` (Ord10887) | `+0x3A740` | **HOOK TARGET**. `__stdcall(pUnit, statId, value, layer)` |
| `STATLIST_SET_STAT` (Ord10261) | `+0x3A280` | Leaf writer at `+0x79` (mov [eax+4],ecx). `__stdcall(pStatList, ?, statId, value)` |
| `STATLIST_SET_BASE_STAT` (Ord10745) | `+0x3A1DB` | 5-arg sibling, `ret 0014`. Same body as Ord10261 + extra pUnit arg push. CE labels mid-function offsets (e.g. `+0x3A240`) as `Ord10745+0x65` — that's correct CE behavior, not a bug. |

#### Stat-list structures

```rust
pub mod stat_list {
    pub const UNIT_TO_STATS_LIST: usize = 0x5C;     // UnitAny → pStatListEx
    pub const SL_PSTAT: usize = 0x24;               // StatListEx → pStat (D2StatStrc*)
    pub const SL_STAT_COUNT: usize = 0x28;          // u16 wStatCount1 (used)
    pub const SL_STAT_CAPACITY: usize = 0x2A;       // u16 wStatCount2 (capacity)
    pub const SL_INLINE_PSTAT: usize = 0x80;        // for monsters: pStat lives inline here

    // D2StatStrc (sizeof = 8)
    pub const STAT_RECORD_SIZE: usize = 8;
    pub const STAT_LAYER: usize = 0x00;             // u16
    pub const STAT_NSTAT: usize = 0x02;             // u16
    pub const STAT_VALUE: usize = 0x04;             // i32

    pub const STAT_HITPOINTS: u16 = 6;
    pub const STAT_MAXHP: u16 = 7;
}
```

#### Internal helper RVAs (informational, only useful if expanding RE)

| Symbol | Approx RVA | Role |
|---|---|---|
| `Ord11013+0xA0` | called from Ord10261+0x72 / Ord10745+0x72 | "remove stat from list" (when new value == 0) |
| `Ord11013+0x1B0` | called from Ord10261+0x24 | "find stat by packed (statId,layer), returns index or -1" |
| `Ord10379+0x2B0` | called from Ord10261+0x81 / Ord10745+0x84 | post-write callback (probably "stat changed" notification) |
| `Ord10871+0x30` | called from Ord10261+0x96 / Ord10887+0x35 | stat-mod insertion (player-only path in Ord10887) |
| `Ord10871+0x120` | called from Ord10261+0x4B | "insert new stat at index" (in not-found-but-value-nonzero branch) |

**`Ord11013+0xA0` value resolved**: from CE during RE session, this maps to `D2Common.dll + 0x381A0` (= Ord11013 entry around `+0x38100`).

### D2Client.dll RVAs

#### Hook-path-relevant

| Symbol | RVA | Notes |
|---|---|---|
| Giant packet handler ("QueryInterface" misnomer) | `+0x4BE70` | Switch-case dispatcher on packet/state ID. **Not our hook target** — we go one frame down — but contains 3 cases that all converge on `Ord10887` |
| Thunk to Ord10887 | `+0xC40C` | `jmp [D2Client.dll+0xCE5FC]` |
| IAT slot for Ord10887 | `+0xCE5FC` | Stores resolved Ord10887 entry = `D2Common.dll` base + `0x3A740`. Exact value drifts per-launch with D2Common rebase. |
| Damage-path caller of `+0x4BE70` | **Not RE'd.** Stack frames seen above the giant switch in the verified damage capture: `+0xD2F00` → `+0x23C2D` → `+0xF21B4` → `+0x2A76D`. (`Ord10003+0x9AF4` showed up only in an unrelated stat-counter capture for stat `0x148`, not in the damage stack — don't conflate.) |

#### Already in `offsets.rs`

| Symbol | RVA | Notes |
|---|---|---|
| `PLAYER_UNIT` | `0x11BBFC` | Pointer to player UnitAny |
| `MERCENARY_UNIT` | `0x10A80C` | |
| `NO_PICKUP_FLAG` | `0x11C2F0` | |
| `INJECT_BASE` | `0xCDE00` | Code-injection scratch area for existing GetStringById path |
| `AUTOMAP_LAYER` | `0x11C1C4` | |
| `func::PRINT_STRING` | `0x7D850` | |
| `func::GET_ITEM_NAME` | `0x914F0` | |
| `func::GET_ITEM_STAT` | `0x560B0` | |
| `func::NEW_AUTOMAP_CELL` | `0x5F6B0` | |

### UnitAny layout (already in offsets.rs)

| Field | Offset | Type |
|---|---|---|
| `dwType` | `+0x00` | u32 (UNIT_PLAYER=0, MONSTER=1, OBJECT=2, MISSILE=3, ITEM=4, TILE=5) |
| `dwClassId` | `+0x04` | u32 |
| `dwUnitId` | `+0x0C` | u32 |
| `pUnitData` | `+0x14` | pointer (for monsters: D2MonsterDataStrc*; `[pUnitData]` = pMonStatsTxtRecord) |
| `pPath` | `+0x2C` | pointer (dynamic path for monsters/players) |
| `pStatListEx` | `+0x5C` | pointer |
| `pInventory` | `+0x60` | pointer (already in offsets.rs::unit::INVENTORY) |
| `pListNext` | `+0xE4` | walks game-wide hash bucket (game-table chain) |
| `pRoomNext` | `+0xE8` | walks per-Room1 unit list |

### MonStats record access pattern

```text
For any monster m:
    p_monstats_record = *((m.pUnitData) + 0)        // pUnitData[0] points to record
    is_spawn          = u8 at  (p_monstats_record + 0x4C)
    max_hp_normal     = u16 at (p_monstats_record + 0xB0)
    max_hp_nm         = u16 at (p_monstats_record + 0xB2)
    max_hp_hell       = u16 at (p_monstats_record + 0xB4)
```

The pUnitData[0] indirection is important — there's NO need to look up the record by class_id from the global pMonStatsTxt table; each monster carries a direct pointer to its record. `record_size * class_id` arithmetic only matters for batch enumeration.

---

## The big function — `D2Client.dll + 0x4BE70`

CE mislabels this as `D2Client.QueryInterface` (probably nearest-export heuristic). It's actually a **giant packet/state dispatcher** — a switch-case function that handles many different game state events triggered by server packets.

The function uses an indirect-jump table at `+0x2596`:
```asm
+2581 lea eax, [edi-0x6E]                    ; edi = packet/event id, normalize range
+2584 cmp eax, 0xF3                           ; check bounds
+2589 ja  +0x2908                             ; out-of-range fallback
+258F movzx ecx, byte [eax + jump_index_table_RVA]
+2596 jmp dword [ecx*4 + jump_targets_RVA]    ; computed jump
```

So it dispatches `~0xF3` (243) cases on packet type. Three of those cases write `STAT_HITPOINTS`:

### Case A — HP percentage update from server (typical damage/healing tick)

Located at `+0x22DD..+0x2325`. **This is the case our breakpoint caught.**

```asm
+22DD test ebx, ebx                           ; ebx = packet payload struct
+22E1 push 0x780                              ;   error fallback if null
+22EB mov ecx, [ebx+04]                       ; flags byte
+22EE and ecx, 0x80                           ; bit 7 of flags
+22F4 push ecx
+22F5 mov eax, ebp                            ; ebp = pUnit
+22F7 call Ord10003+0xF3E0                   ; helper (probably "update HP UI?")
+22FC mov edx, [ebx+04]                       ; reload flags
+22FF and edx, 0x7F                           ; ★ low 7 bits = HP percentage (0-127)
+2302 mov eax, edx
+2304 cmp eax, 1                              ; if (hp_pct > 1) increment
+2307 mov [ebx+04], edx
+230A jle +0x2310
+230C inc eax                                 ; off-by-one: 2..127 → 3..128
+230D mov [ebx+04], eax
+2310 mov eax, [ebx+04]                       ; reload final value
+2313 test eax, eax
+2315 je  +0x2325                             ; skip if 0 (handled by case C)
+2317 push 00                                 ; layer = 0
+2319 shl eax, 8                              ; ★ value = hp_pct * 256 (= 0..32768)
+231C push eax                                ; value
+231D push 06                                 ; statId STAT_HITPOINTS
+231F push ebp                                ; pUnit
+2320 call D2Client+0xC40C                    ; → thunk → Ord10887
+2325 ; resume
```

**This is where the 32768 normalization originates.** The packet contains a 7-bit percentage; the client scales it locally to fit the engine's stat-value space.

### Case B — alternate HP update path

Located at `+0x2376..+0x238B`. Same pattern but reads the value from a different field of the packet payload (`[ebx+0x14]` instead of `[ebx+0x04]`):

```asm
+2376 mov eax, [ebx+0x14]                     ; different field
+2379 test eax, eax
+237B je  +0x238B
+237D push 00                                 ; layer
+237F shl eax, 8                              ; same scaling
+2382 push eax
+2383 push 06
+2385 push ebp
+2386 call D2Client+0xC40C                    ; → Ord10887
+238B ; resume
```

Probably the "monster mana update" or a different damage-event format (possibly initial spawn, or critical-strike report).

### Case C — kill / clear HP

Located at `+0x2472..+0x247C`. Sets HP to zero — probably the "monster died" event:

```asm
+2472 push esi                                ; esi was xor'd to 0 earlier
+2473 push esi                                ;   (so this pushes 0, 0)
+2474 push 06                                 ; statId
+2476 push ebp                                ; pUnit
+2477 call D2Client+0xC40C                    ; → Ord10887(pUnit, 6, 0, 0)
+247C ; resume
```

### Why hooking case A specifically would be wrong

You'd miss cases B and C. Hooking **`Ord10887`** catches all three uniformly, plus any other future caller (D2Game's own writes in SP, item-mod recompute, anything).

---

## The hook target — `Ord10887` full disasm

`D2Common.dll + 0x3A740`:

```asm
Ord10887:
+00  8B 44 24 0C    mov eax, [esp+0xC]        ; eax = arg2 = value
+04  53             push ebx
+05  8B 5C 24 0C    mov ebx, [esp+0xC]        ; ebx = arg1 = statId
+09  56             push esi
+0A  8B 74 24 0C    mov esi, [esp+0xC]        ; esi = arg0 = pUnit
+0E  8B 4E 5C       mov ecx, [esi+0x5C]       ; ecx = pUnit->pStatListEx
+11  57             push edi
+12  8B 7C 24 1C    mov edi, [esp+0x1C]       ; edi = arg3 = layer
+16  57             push edi                  ; ┐
+17  50             push eax                  ; │ → call Ord10261(
+18  53             push ebx                  ; │     pStats, statId, value, layer)
+19  51             push ecx                  ; ┘
+1A  E8 21FBFFFF    call Ord10261             ; (jumps back +5 = -0x4DF, lands at 0x3A280)
+1F  85 C0          test eax, eax             ; ← return-to address (CE labels this point)
+21  74 17          je +3A
+23  83 3E 00       cmp dword [esi], 0        ; if (pUnit->dwType == UNIT_PLAYER)
+26  75 12          jne +3A
+28  8B C3          mov eax, ebx              ; player branch:
+2A  8B 5E 5C       mov ebx, [esi+0x5C]       ;   reload pStats
+2D  0FB7 D7        movzx edx, di             ;   layer (low 16 of edi)
+30  C1 E0 10       shl eax, 0x10             ;   eax = statId << 16
+33  03 C2          add eax, edx              ;   eax = (statId << 16) | layer
+35  E8 D6DDFFFF    call Ord10871+0x30        ;   stat-mod insertion
+3A  5F             pop edi
+3B  5E             pop esi
+3C  5B             pop ebx
+3D  C2 1000        ret 0010                  ; cleans 4 stack args
```

**Calling convention:** `__stdcall`, callee cleans 16 bytes (4 args).

**Args at entry (before any prologue pushes):**
- `[esp+4]` = `pUnit`
- `[esp+8]` = `statId`
- `[esp+0xC]` = `value`
- `[esp+0x10]` = `layer`

Matches D2MOO `STATLIST_SetUnitStat(D2UnitStrc* pUnit, int nStatId, int nValue, uint16_t nLayer)` 1:1.

**Inline-hook target byte sequence (5 bytes, replaceable with `E9 rel32 = jmp trampoline`):**
```
8B 44 24 0C 53
└─ mov eax,[esp+0xC] ─┘└─ push ebx ─┘
```

Restoring these 5 bytes in the trampoline before jumping back to `Ord10887+5` is straightforward — no `eip`-relative instructions in this prefix, no relocation problems.

---

## The leaf writer — `Ord10261` full disasm

`D2Common.dll + 0x3A280`. Only relevant for understanding the call chain — we don't hook here.

```asm
Ord10261:
+00  53             push ebx
+01  8B 5C 24 08    mov ebx, [esp+8]          ; ebx = arg0 = pStatList
+05  85 DB          test ebx, ebx
+07  75 06          jne +0x0F                 ; null-check
+09  33 C0          xor eax, eax
+0B  5B             pop ebx
+0C  C2 1000        ret 0010                  ; null path

+0F  0FB7 44 24 14  movzx eax, word [esp+0x14]  ; eax = arg3 low 16 = layer
+14  56             push esi
+15  57             push edi
+16  8B 7C 24 14    mov edi, [esp+0x14]       ; edi = arg1 = statId (after 3 pushes)
+1A  C1 E7 10       shl edi, 0x10             ; edi = statId << 16
+1D  03 F8          add edi, eax              ; edi = (statId << 16) | layer (packed key)
+1F  8D 73 24       lea esi, [ebx+0x24]       ; esi = &pStatList->pStat
+22  8B C6          mov eax, esi
+24  E8 07E0FFFF    call Ord11013+0x1B0      ; eax = find_stat_index(&pStat, packed_key)
+29  85 C0          test eax, eax
+2B  7C 09          jl +0x36                  ; not found → branch
+2D  8B 0E          mov ecx, [esi]            ; ecx = pStatList->pStat (array)
+2F  8D 04 C1       lea eax, [ecx + eax*8]    ; eax = &pStat[index]
+32  85 C0          test eax, eax
+34  75 1A          jne +0x50                 ; found existing → write path

+36  8B 44 24 18    mov eax, [esp+0x18]       ; arg2 = value
+3A  85 C0          test eax, eax
+3C  75 08          jne +0x46                 ; new stat with value != 0
+3E  5F             pop edi
+3F  5E             pop esi
+40  33 C0          xor eax, eax              ; not found, value=0 → no-op
+42  5B             pop ebx
+43  C2 1000        ret 0010

+46  8B 13          mov edx, [ebx]            ; insert-new-stat path
+48  52             push edx
+49  8B C7          mov eax, edi
+4B  E8 70E3FFFF    call Ord10871+0x120       ; eax = newly inserted pStat record

+50  8B 48 04       mov ecx, [eax+04]         ; ecx = currentValue
+53  55             push ebp
+54  8B 6C 24 1C    mov ebp, [esp+0x1C]       ; ebp = newValue
+58  2B E9          sub ebp, ecx              ; ebp = new - cur (delta)
+5A  75 09          jne +0x65                 ; if delta != 0
+5C  5D             pop ebp
+5D  5F             pop edi
+5E  5E             pop esi
+5F  33 C0          xor eax, eax              ; no change → return false
+61  5B             pop ebx
+62  C2 1000        ret 0010

+65  8B 4C 24 1C    mov ecx, [esp+0x1C]       ; ecx = newValue
+69  85 C9          test ecx, ecx
+6B  75 0C          jne +0x79                 ; if new != 0, write
+6D  8B 0B          mov ecx, [ebx]            ;   else "remove" path:
+6F  51             push ecx
+70  8B D0          mov edx, eax
+72  E8 A9DEFFFF    call Ord11013+0xA0        ;   remove stat from list
+77  EB 03          jmp +0x7C
+79  89 48 04       mov [eax+04], ecx         ; ★ THE WRITE (the famous one)

+7C  6A 00          push 00
+7E  55             push ebp                  ; (delta from +0x58)
+7F  8B C3          mov eax, ebx
+81  E8 6AFCFFFF    call Ord10379+0x2B0       ; "stat changed" callback
+86  8B 43 10       mov eax, [ebx+0x10]
+89  85 C0          test eax, eax
+8B  79 0E          jns +0x9B
+8D  8B 43 08       mov eax, [ebx+0x08]
+90  85 C0          test eax, eax
+92  75 07          jne +0x9B
+94  8B C7          mov eax, edi              ; eax = packed (statId, layer)
+96  E8 35E2FFFF    call Ord10871+0x30        ; conditional stat-mod insertion
+9B  5D             pop ebp
+9C  5F             pop edi
+9D  5E             pop esi
+9E  B8 01000000    mov eax, 1                ; return TRUE
+A3  5B             pop ebx
+A4  C2 1000        ret 0010
```

**Note on Ord10261 vs Ord10745**: There's a sibling function `Ord10745` (at `+0x3A1DB`) with **identical body but `ret 0014`** (5 args). The 5th arg is an extra `pUnit` reference used for additional callbacks. Mid-function offsets (e.g. `+0x3A240`) are correctly labeled by CE as `Ord10745+0x65`; if you see that label, you're inside Ord10745's body, not at any function entry.

---

## Decisions log (compressed history)

### 1. Raw `dwValue`, no `>>8` shift (decision early in RE)

Use `dwValue as u32` directly instead of `>>8`. Numbers are 256× bigger; same proportionality, more presentable. This was a pre-pivot decision when we still thought we'd polling-style read HP. **Still applies** to any `read_unit_stat_direct` helper we keep.

### 2. Summon filter: `MonStats.is_spawn` (verified)

`MonStats.txt + 0x4C` (u8): 0=wild, 1=spawnable. Verified across 7 wild + 5 summon classes. **Still the way** to filter out summon damage in our DPS hook.

### 3. Plan B math — RESURRECTED

Originally rejected (decision #5 in old version of this doc) because:
- Engine HP normalized to 32768 → small numbers
- Polling missed one-shot kills
- Reconcile heuristic created false positives

**All three problems are dissolved by the hook approach + protocol-quirk discovery:**
- 32768 IS `(127 << 8 + 1<<8) = 0x8000` — server protocol says HP percent → client multiplies. Therefore `delta_raw / 32768` is a TRUE percentage of full HP.
- `MonStats.wMaxHP[difficulty]` is the per-monster TRUE max HP (template value). `damage = (delta_raw / 32768) × monstats_max` gives **engine-honest absolute damage**.
- Hook fires on every write, no polling-tick-misses.
- Kills are events with `value == 0`, not heuristic disappearances.

### 4. Hook target = `Ord10887`, NOT `Ord10261` and NOT D2Game's `SUNITDMG_*`

- `SUNITDMG_ExecuteEvents` (D2Game.dll): server-side, doesn't run in MP client process. **REJECTED**.
- `Ord10261` (D2Common): leaf writer, fires for every stat, including non-monster contexts. Too noisy.
- `Ord10887` (D2Common): **CHOSEN**. Top-level public API for "set unit stat", takes `pUnit` directly, runs in both SP and MP, single call site catches all paths.

### 5. Level chain (auto-reset) — RESOLVED 2026-05-08

`Room1+0x40 → Room2+0x90 → Level+0xD8` from D2MOO returned null Room2 in MXL.
Replaced with the verified MXL Sigma chain:

```text
dwLevelNo = *(*(*(pRoom1 + 0x10) + 0x24) + 0x100)
```

Verified live across three areas (Rogue Encampment=1, Dark Wood=5,
MXL Sigma "Outer Cloister"=166). See **Round 2 RE** section below
and `offsets.rs::level_chain`.

This unblocks **auto-reset on area change** for v1 — was originally
deferred to v2 in the spec, now in scope.

`docs/ce-scripts/verify-level-chain.lua` retained as the verification
probe that found the chain.

### 6. UnitAny+0x88 / UnitAny+0x10 / pPath+0x20 — all wrong leads (RE deferred)

Multiple attempts at finding a per-unit owner/team/minion field came up empty. Conclusion: D2's owner/pet tracking is centralised (PetManager), not per-unit. Filter via `MonStats.is_spawn` instead. No further RE needed for DPS purposes.

---

## Round 2 RE — Level chain + Difficulty (2026-05-08 / 2026-05-09)

Two extras that were "deferred / small TODO" in the original doc were
RE'd during the implementation prep so they could ship in v1.

### Level chain — `dwLevelNo` from `pRoom1`

D2MOO vanilla `Room1+0x40 → Room2+0x90 → Level+0xD8` is broken in MXL
Sigma — `Room1+0x40` is NULL. Round-2 brute-force found that
`Room1+0x100` is just the next Room1 in a 0x100-byte heap pool (not a
levelId), and the OOB chain `(0x0, 0x40, 0x100) = 5/1` in DW/RE was a
coincidence — values bled in from adjacent heap allocations.

Final chain — verified across three areas (Rogue Encampment=1,
Dark Wood=5, MXL "Outer Cloister"=166):

```text
dwLevelNo (u32) = *(*(*(pRoom1 + 0x10) + 0x24) + 0x100)
```

`*(pRoom1 + 0x10)` is likely the MXL analogue of `Room2`;
`*(... + 0x24)` is likely the MXL analogue of `pLevel`. We don't
need to name them — the byte-offset chain is what the scanner walks.

Constants in `offsets.rs::level_chain`:
- `ROOM1_TO_INTERMEDIATE = 0x10`
- `INTERMEDIATE_TO_LEVEL = 0x24`
- `LEVEL_TO_LEVEL_NO = 0x100`

### Difficulty static

Resolved by 2-pass cross-run diff: scan `D2Client.dll +0x000000..+0x180000`
for `u32 == 0` (on Normal), restart on Nightmare and filter to
`u32 == 1` — narrowed 12 200+ candidates to **exactly one** RVA.

```text
dwDifficulty (u32) = *(D2Client.dll + 0x11C390)
```

`0 = Normal`, `1 = Nightmare`, `2 = Hell`. Used for absolute-damage
scaling: `damage = (delta_raw / 32768) * MonStats.wMaxHP[difficulty]`.

Added to `offsets.rs::d2client::DIFFICULTY = 0x11C390`.

`docs/ce-scripts/find-difficulty.lua` retained — the same script can
re-RE the static if MXL Sigma updates (it's a simple two-pass scan).

---

## CE-script artifacts

All in `docs/ce-scripts/`:

| Script | Purpose | Status |
|---|---|---|
| `verify-dps-meter-offsets.lua` | Phase 1 — dump StatList + Level chain for one monster | ✅ used in early RE |
| `dump-stats.lua` | Full stat dump for player + target monster | ✅ used |
| `find-owner-field.lua` | Search UnitAny + pUnitData for owner/minion field | ✅ used (negative result, see decision #6) |
| `find-team-field.lua` | Search pPath for team field | ✅ used (negative result) |
| `verify-level-chain.lua` | Directed probe of Room1+0x10 / +0x08 / pPath chains for `dwLevelNo` | ✅ used (Round 2 RE, found chain) |
| `verify-monstats.lua` | wId-fingerprint approach to find pMonStatsTxt | superseded |
| `find-monstats-via-monster.lua` | Live-monster triangulation: scan pUnitData for ptrs | ✅ used (the one that worked) |
| `dump-monstats-records.lua` | Diff records of wild vs summon classes to find isSpawn + HP fields | ✅ used |
| `find-apply-damage.lua` | Locate target monster's stat 6 dwValue address for CE write-watchpoint workflow | ✅ used (the script that found the chain) |
| `find-difficulty.lua` | Two-pass cross-run diff to find `dwDifficulty` static | ✅ used (Round 2 RE, narrowed 12k+ → 1) |

Each script is self-documented at the top with how-to-run instructions.

---

## Game state context for sanity checks (when resuming)

Reference values for sanity-checking a fresh CE session:

- Player `unit_id` = 1
- Player UnitAny varies per run (heap allocation): `0x024EE900` / `0x023E1E900` / `0x02BAAE900` etc.
- Player real HP from globe: 11550 (lvl 134 Sorc); stat 6 raw read: 2791680 (`>>8` = 10905, slight delta from regen between scans is normal)
- Treehead full HP: stat 6/7 raw = **32768** (constant across all monsters in MXL — protocol quirk, see TL;DR)
- Monster pUnit examples seen: Treehead = 0x049BDA00 (varies)
- StatListEx examples seen: 278F6080 / 278FDA80 (varies — heap reallocates on stat-count changes, ~5-15× per session)

Confirmed monster class ids encountered:
- **Wild**: 2881 (Treehead boss), 3131-3143 (Tree Ent variants in Hell Dark Wood), 2846-2849 (Blood Moor mobs in Normal)
- **Summons**: 1061, 1062 (suicide bomber), 1063, 1103, 1602, 1691

Fallback heuristic if MonStats RE re-fails: `class_id < 2000` covers all observed summon classes. **Don't use this as primary filter** — `is_spawn` from MonStats record is engine truth.

---

## Resuming work — checklist for next session

The implementation in next session can proceed without re-running CE if heap addresses don't change semantics. RVAs are stable.

1. **Read this doc, the spec, the plan.** Don't re-derive — trust the verified offsets.

2. **`offsets.rs` is already populated** (Task 1 of plan was done alongside RE):
   - `d2common::STATLIST_SET_UNIT_STAT = 0x3A740`
   - `d2common::STATLIST_SET_STAT = 0x3A280`
   - `stat_list` module with all `D2StatListEx` + `D2StatStrc` field offsets
   - `d2client::DIFFICULTY = 0x11C390` (for `wMaxHP[difficulty]` scaling)
   - `level_chain` module (auto-reset on area change, was deferred to v2 — now in v1 scope)

3. **Implement** per `docs/superpowers/plans/2026-05-07-dps-meter.md`. Plan is structured for subagent-driven-development.

4. **Smoke-test in-game** with three scenarios:
   - **Player melee** — confirms basic damage capture
   - **Pure summoner** — player stands still, pets fight, confirms the universal-sink claim (events fire regardless of attacker)
   - **One-shot kill** — confirms `value==0` event captures the kill cleanly

5. **Don't forget the DoT caveat**. Poison/Burn writes go to `STAT_HPREGEN` (a different stat), NOT `STAT_HITPOINTS`. Our hook on stat 6 will NOT capture DoT damage at application time — the regen-tick handler later spreads regen into stat 6 separately, which our hook DOES catch. Net effect: **DoT damage is captured but with a tick delay**. Acceptable for v1; document in UI tooltip.

6. **MP-compatibility** is the hard requirement (per user). Test on a realm during smoke test, not just SP. Confirm `D2Game.dll` IS or ISN'T loaded — doesn't matter for our hook (we don't touch D2Game).

---

## Open issues / future work

### Multi-write-per-event handling

When stat 6 is updated, `Ord10261+0x81` calls `Ord10379+0x2B0` (post-write callback) which may internally trigger further stat writes (e.g., recompute derived stats). Our hook fires on each `Ord10887` call, so we naturally catch one entry per packet event — but if a derived stat write loops back through `Ord10887`, we might double-count.

**Mitigation**: in our hook handler, deduplicate by checking `now_ms - last_event_for_unit_ms < 16` (one game tick). If too close, treat as same event. Or: simpler — only count writes that came from the D2Client side (check return address up the stack is in D2Client.dll range). Defer until smoke test reveals if this is an actual problem.

### Non-percentage HP packets

Cases A/B in the QueryInterface dispatcher both feed an `eax << 8` value. **What if there's a packet type that sends absolute HP, not percentage?** Bosses with HP > 32768 might require this. We haven't enumerated all 243 dispatcher cases.

**Mitigation**: in the hook, when reading `value`, sanity-check `value <= 32768`. If higher, log a warning and don't try to scale via MonStats. Defer enumerating other cases until we see one that breaks.

### MXL custom stat 0x148 (= 328)

Discovered during a misfired CE breakpoint: there's a function at `D2Client.dll + 0x2A96D` that increments stat 0x148 by 1 on certain events. Probably a kill counter or activity tracker. **Not directly relevant** to DPS metric — but documented here in case it becomes useful for a future "kills/min" or "engagement timer" feature.

### `Ord10745` 5-arg sibling

`Ord10745` entry is at `D2Common.dll+0x3A1DB`. It has the same body as `Ord10261` but with `ret 0014` (5 args, extra `pUnit`). It's **not** in our hook path — calls from D2Client go through `Ord10887 → Ord10261`. Possibly `Ord10745` is for D2Game-internal use in SP.

**Action**: ignore unless it shows up in a future RE pass.

---

## Appendix — full RE timeline (compressed)

For curious next-session-readers who wonder how we got here:

1. **StatList chain RE** — ✅ verified `UnitAny+0x5C → StatListEx → +0x24/+0x28` chain, found inline-pStat-at-+0x80 quirk for monsters
2. **MXL HP scaling discovery** — observed 32768 max for ALL monsters, originally interpreted as MXL design choice
3. **Decision: HP-delta polling** — rejected because no absolute scale (we hadn't yet RE'd MonStats max)
4. **MonStats RE** — ✅ found `pMonStatsTxt` at `sgptDataTables + 0xA78`, fields verified, gave us per-difficulty max HP
5. **Pivot 1: `D2Game::SUNITDMG_ExecuteEvents`** — initially planned, then rejected because D2Game.dll runs server-side in MP and isn't hookable from client
6. **Pivot 2: client-side path** — CE breakpoint on stat 6 dwValue revealed call stack stays in D2Client → D2Common. **MP-compatibility unlocked.**
7. **Identified D2Client+0x4BE70 packet dispatcher** — giant switch with 3 stat-6-writing cases
8. **Identified `Ord10887` as universal sink** — single hook covers all 3 cases
9. **Protocol-quirk discovery** — analysing case A revealed the 0-127 percentage protocol → 32768 normalization explained
10. **Plan B resurrection** — formula `(delta_raw / 32768) × monstats_max_hp` gives absolute damage
11. **Full disasm of `Ord10887` + `Ord10261`** — verified hook is feasible (5-byte hot-patchable prologue)
12. **RE COMPLETE** ← current state

This trail of pivots was the right way to find the truth — each rejected hypothesis ruled out a class of mistakes for the implementation.
