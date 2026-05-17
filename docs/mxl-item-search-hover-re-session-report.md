# MXL Item Search Hover RE Session Report

Date: 2026-05-16

## Goal

Find a reliable way to identify the item currently under the native Diablo II / Median XL UI tooltip so the item-search overlay can pre-fill the exact hovered item without relying on fragile screen-coordinate inventory-grid mapping.

The original fallback from `docs/superpowers/plans/2026-05-15-mxl-item-search.md` was:

```text
mouse position -> D2 client coordinates -> UI grid rect/cell -> pInventory->pGrids -> UnitAny*
```

The user explicitly does not trust this as the primary approach because items have different sizes and UI coordinates are resolution/window/layout dependent. This session therefore focused on finding a more native source of truth.

## High-Level Outcome

No stable, directly pollable global hover pointer was found.

The best confirmed source of truth is a native D2Sigma tooltip hook:

```text
D2Sigma.dll+AE020 entry
[ESP+04] = current tooltip item UnitAny*
```

This hook fires repeatedly while the tested item tooltip is visible, stops on empty hover, and covers the checked equipment/inventory/cube/stash UI slots. It avoids mouse-grid identity entirely because the game/mod has already decided which item is being used to build the native tooltip.

Other useful RE outputs:

```text
1. Native tooltip text buffer around 0x00194080..0x00194280.
2. Earlier D2Sigma tooltip paths where current item UnitAny* appears at D2Sigma.dll+65970 [ESP+0C], D2Sigma.dll+6927B ESI, and D2Sigma.dll+6973B ESI.
3. Stale/transient pointer cache copies around 0x001940DC, 0x00194468, 0x001944B0, and 0x001944EC.
```

Recommended implementation direction: install a runtime hook/trampoline at `D2Sigma.dll+AE020`, validate `[ESP+04]` as an item `UnitAny*`, and save it with a timestamp/sequence freshness guard for the item-search hotkey.

## Confirmed Local Inventory Path

The existing player inventory path remains valid and was used to obtain known item `pUnit` values for search experiments:

```text
D2Client.dll+11BBFC -> UnitAny* player
UnitAny + 0x60      -> Inventory*
Inventory + 0x0C    -> UnitAny* first item
Inventory + 0x14    -> D2InventoryGridStrc* grids
grid + 0x0C         -> UnitAny** ppItems
```

Observed example from an inventory with two visible items:

```text
player    = 25438D00
inventory = 2ED119C0

grid[02] inventory, 15x10, ppItems=03CECC00
item A: pUnit=25439A00 class=00000418 id=00000006 cells=0,1,15,16,30,31 size=2x3
item B: pUnit=25439C00 class=0000014D id=00000007 cells=35,36,50,51,65,66 size=2x3
```

This path is useful for validation and fallback, but should not be treated as the preferred identity source if a native tooltip path is available.

## Rejected Direct Hover Pointer Candidates

### Known 1.13c / D2Stats-Style Selected Item Globals

These stayed zero while hovering different UI items:

```text
D2Client.dll+11BC38 = 00000000
D2Client.dll+11B6D4 = 00000000  ; predicted from 1.13d SelectedInvItem delta
```

The `D2Client.dll+11B6D4` prediction came from comparing the Frozen Keep 1.13d offsets with the known MXL player-unit offset:

```text
1.13d PlayerUnit:      D2Client+11D050
MXL PlayerUnit:        D2Client+11BBFC
delta:                 -0x1454

1.13d SelectedInvItem: D2Client+11CB28
predicted MXL:         D2Client+11B6D4
```

Result: not usable.

### Transient Last-Hover Pointer Copies

Searching known `pUnit` values while hovering items found four addresses:

```text
001940DC
00194468
001944B0
001944EC
```

They changed to the last item hovered in inventory/stash, but they did not clear on empty hover and produced invalid values for some equipment cases.

Example behavior:

```text
hover Shamanka/other item -> candidate value becomes that item UnitAny*
hover empty cell          -> value remains last hovered item
hover equipment           -> mixed zero / small integers / garbage
```

`Find out what writes` showed ordinary stack/transient writes, not stable globals:

```text
push ebp
push edi
push eax
push -01
movdqu [edi],xmm0
jmp dword ptr [...]
```

Conclusion: these addresses are RE evidence for a tooltip/hover path, but must not be read directly in production.

### Manual Hover Flag Search

An address such as `001AED9C` was found during manual experiments. It showed `1` while hovering an item, but random/non-deterministic values in other cases.

Conclusion: not a reliable boolean flag.

## Frozen Keep 1.13d Offset Checks

The Frozen Keep thread `https://d2mods.info/forum/viewtopic.php?f=8&t=69575` lists 1.13d functions/globals. Direct and delta-adjusted globals were tested.

### D2Client Globals

Module base in this session:

```text
D2Client.dll base = 6FAB0000
```

Checked candidates:

```text
known_mxl_player       D2Client+11BBFC value=258B8D00  ; valid player UnitAny*
old_113c_selected      D2Client+11BC38 value=00000000
predicted_selected     D2Client+11B6D4 value=00000000
predicted_mouse_x      D2Client+11B4FC value=00000000
predicted_mouse_y      D2Client+11B4F8 value=00000000
predicted_cursor_x     D2Client+0ED058 value=00000000
predicted_cursor_y     D2Client+0ED05C value=00000000
```

Conclusion: the simple delta only matched `PlayerUnit`; it does not make these globals portable to this MXL build.

### Layout Globals

The following direct and delta-adjusted layout globals were all zero:

```text
inventory_113d   D2Client+1016F0 = 00000000
stash_113d       D2Client+1015E0 = 00000000
cube_113d        D2Client+1016D8 = 00000000

inventory_delta  D2Client+10029C = 00000000
stash_delta      D2Client+10018C = 00000000
cube_delta       D2Client+100284 = 00000000
```

Conclusion: Frozen Keep layout globals are not directly useful in this build.

### Function Offset Guesses

Function candidates were guessed by applying the `GetItemName` delta:

```text
1.13d GetItemName: D2Client+958C0
MXL GetItemName:   D2Client+914F0
delta:             -0x43D0
```

Checked candidates:

```text
GetCursorItem   -> D2Client+100D0
GetSelectedUnit -> D2Client+12EB0
LeftClickItem   -> D2Client+96C20
ClickItemRight  -> D2Client+98B60
```

`ClickItemRight` looked syntactically plausible and contained grid-like math, but breakpoints on:

```text
D2Client+98B60
D2Client+98B88
D2Client+98BAC
```

did not fire on inventory/stash/cube clicks. This candidate is not the active click path for the tested actions.

## Legacy D2Stats Tooltip Text Path

The legacy AutoIt source uses tooltip text rather than a selected-item pointer for copy-item-text:

```autoit
$sOutput = _MemoryPointerRead($g_hD2Win + 0x1191F, $g_ahD2Handle, $aiOffsets, "wchar[8192]")
;$sOutput = _MemoryRead(0x00191FA4, $g_ahD2Handle, "wchar[2048]") ; Magic?
```

This was tested in the current MXL build.

### D2Win+1191F

Module base:

```text
D2Win.dll base = 6F8E0000
D2Win.dll+1191F = 6F8F191F
```

Observed:

```text
root=6F8F191F
a1=6B1860A8
```

Direct and nearby pointer reads returned unrelated text/gibberish, including ASCII strings about Cain when interpreted as UTF-16. This does not point to current item tooltip text in this build.

### Legacy Magic Address

`0x00191FA4` was empty or not useful.

Conclusion: the exact legacy pointer path is stale, but the legacy strategy of reading a tooltip text buffer is still valid.

## Native Tooltip Text Buffer Found

Searching for actual visible tooltip text found a live UTF-16 buffer in the `0x00194080..0x00194280` range.

Example: searching for visible `Shamanka` produced a relevant wide hit:

```text
00194112
```

Scanning nearby wide strings and scoring them found full tooltip starts:

```text
Shamanka:
best_addr=001940E4
text=ÿc4Long Staff (Sacred)\nShamanka

Arrogance:
best_addr=001940F0
text=ÿc4Gothic Plate (Sacred)\nArrogance
```

The buffer uses Diablo color codes such as `ÿc4` and UTF-16/wchar text.

### Tooltip Buffer Staleness

On empty hover, the buffer remained stale:

```text
empty:
best_addr=001940F0
text=ÿc4Gothic Plate (Sacred)\nArrogance
```

Conclusion: this buffer is excellent for copy-item-text while the user is visibly hovering an item, but it is not sufficient by itself for target identity because it can remain stale after leaving the item.

## D2Sigma Tooltip Builder Paths

Manual access tracing from suspicious hover-related stack data first led to:

```text
D2Sigma.dll+65970
```

Module base in this session:

```text
D2Sigma.dll base = 6AAE0000
D2Sigma.dll+65970 = 6AB45970
```

The function looks like a generic container/list copy/build helper:

```asm
D2Sigma+65970  push ebp
D2Sigma+65971  mov ebp,esp
...
D2Sigma+65998  push 44
D2Sigma+659AA  call D2Sigma+13395D     ; allocate 0x44-byte node
...
D2Sigma+65A1C  movups xmm0,[esi]
D2Sigma+65A2F  movups [ecx+10],xmm0
...
D2Sigma+65A68  add esi,34             ; source record step = 0x34
```

The function itself is not a hover pointer or flag. The useful part is its stack state during tooltip construction.

### Confirmed pUnit On Stack

Breakpoint hits at `D2Sigma+65970` while an item tooltip was being built showed:

```text
RET=6AB45F39
ECX=001AF6A4
ARG1=001AF4B0
ARG2=001AF5E8
[esp+0C]=252C4E00
```

Validating `[esp+0C]` as `UnitAny*`:

```text
UNIT CHECK 252C4E00
type=00000004
class=0000012A
id=0000007A
mode=00000000
data=0FFBD400
inv=00C4C200
```

This confirms that the current tooltip item `UnitAny*` is available at `[esp+0C]` when `D2Sigma+65970` is entered.

Other hits in the same sequence returned nearby callers:

```text
RET=6AB45F39
RET=6AB4605F
RET=6AB4618A
RET=6AB462C0
RET=6AB463DD
RET=6AB46513
RET=6AB4665F
RET=6AB467AB
```

The caller around `D2Sigma+65EF0` contains several calls to `D2Sigma+65970`, for example:

```asm
6AB45F34 - call 6AB45970
6AB4605A - call 6AB45970
```

The caller builds several temporary record ranges using local stack-frame storage. The current item `pUnit` is not obviously loaded next to these call instructions, so `D2Sigma+65970` entry remains the simplest confirmed hook point.

### Final Hook Candidate: D2Sigma+AE020

Later filtered hook-site scouting found a better, more direct tooltip hook:

```text
D2Sigma.dll+AE020
```

Final observed module base in the coverage session:

```text
D2Sigma.dll base = 68AB0000
D2Sigma.dll+AE020 = 68B5E020
```

On entry:

```text
[ESP+04] = current tooltip item UnitAny*
```

Confirmed prologue and patch contract:

```asm
D2Sigma.dll+AE020  55                 push ebp
D2Sigma.dll+AE021  8D 6C 24 D8        lea ebp,[esp-28]
D2Sigma.dll+AE025  83 EC 28           sub esp,28
D2Sigma.dll+AE028  6A FF              push -01
D2Sigma.dll+AE02A  68 3912C168        push D2Sigma.dll+161239
```

```text
patch address: D2Sigma.dll+AE020
patch size:    5 bytes
saved bytes:   55 8D 6C 24 D8
resume at:     D2Sigma.dll+AE025
```

If the trampoline begins with `pushfd; pushad`, the original hook argument moves from `[ESP+04]` to `[ESP+28]` while registers/flags are saved.

Validation shape for `[ESP+04]`:

```text
[pUnit + 0x00] == 4        ; item UnitAny
[pUnit + 0x04] <  0x10000  ; plausible class id
[pUnit + 0x10] != 3        ; reject ground item mode for this feature
[pUnit + 0x14] != 0        ; ItemData*
```

Coverage observed with `docs/ce-scripts/mxl-tooltip-ae020-coverage.lua`:

```text
Paragon's Hammer: maxGap=234ms
Horadric Cube:    maxGap=31-47ms
Amulets:          maxGap=16-32ms
Shamanka:         maxGap=32ms
Catalyst:         maxGap=31-46ms
Hunter's Bow:     maxGap=16ms
```

Manual confirmation:

```text
empty hover -> no hits
checked UI slots -> current tooltip item UnitAny* at [ESP+04]
```

This is better than `D2Sigma+65970` because the current tooltip item is the first stack argument, coverage was explicitly checked across several UI slots, and empty hover naturally produces no fresh hits. `D2Sigma+65970`, `D2Sigma+6927B`, and `D2Sigma+6973B` remain useful fallback hook candidates if `AE020` is version-specific or fails in a future build.

## Current Best Technical Options

### Option 1: D2Sigma+AE020 Hook For Native Tooltip Item

Install a runtime hook/trampoline at `D2Sigma.dll+AE020`. On entry:

```text
candidate = [esp+04]
if candidate is valid UnitAny item:
  [candidate+0x00] == 4
  [candidate+0x14] != 0
  [candidate+0x10] != 3
then write candidate and timestamp/sequence into allocated shared memory
```

Hotkey behavior:

```text
read saved pUnit + timestamp
accept only if timestamp is fresh, e.g. within 500-750 ms
use pUnit for native item name/stats lookup or use tooltip text buffer for exact rendered text
```

Pros:

- Does not depend on mouse coordinates, grid cell size, item dimensions, window size, or UI layout.
- Uses the game/mod's own tooltip construction as source of truth.
- Can solve stale tooltip buffer if timestamp is fresh only while tooltip is actively being rebuilt.

Cons:

- Requires patching executable code in `D2Sigma.dll` at runtime.
- Needs a safe trampoline/code cave and byte restoration on shutdown.
- Needs careful validation and crash handling.
- Needs version/build guardrails because this is a module-relative code hook.

Implementation guard:

```text
Resolve D2Sigma.dll base each session.
Patch only D2Sigma+AE020 for the known supported build.
Accept the saved pUnit only when the last hook hit is fresh.
Clear or ignore stale state after no hits for the freshness window.
```

### Option 2: Tooltip Text Buffer With Stale Guard

Read best-scored UTF-16 string from `0x00194080..0x00194280`, strip Diablo color codes, and use the last non-empty line as the item search name or full reversed text for copy-item-text.

Pros:

- Very close to the legacy D2Stats behavior.
- Provides native rendered item text including stats.
- Low implementation complexity compared with a hook.

Cons:

- Buffer is stale on empty hover.
- Needs an independent validity guard.

Possible guards:

```text
1. D2Sigma hook freshness timestamp.
2. A yet-to-be-found tooltip visible/current flag.
3. pInventory->pGrids mouse hit-test, only as a stale guard, not as the source of item identity.
```

### Option 3: pInventory->pGrids Mouse Hit-Test

Use mouse/client coordinates to resolve the inventory/cube/stash/equipment cell and read `ppItems[cell]`.

Pros:

- Already confirmed readable.
- Non-invasive: no code patching.
- Correctly rejects empty cells if mapping is right.

Cons:

- User distrusts it as primary identity, reasonably, because UI geometry and multi-cell items make this fragile.
- Requires calibration per UI/window/resolution/layout.
- Does not naturally provide full native tooltip text.

Recommendation: keep only as fallback or stale guard if the project avoids hooks.

## Suggested Implementation Plan

### Step 1: Hook D2Sigma+AE020

Install a small runtime trampoline at `D2Sigma.dll+AE020` after resolving the module base for the current Diablo II process.

Hook contract:

```text
on D2Sigma+AE020 entry:
  candidate = [ESP+04]
  if candidate validates as UnitAny item:
    save candidate
    save tick timestamp
    increment sequence
  run original overwritten bytes
  jump back to D2Sigma+AE020 + overwritten_len
```

Concrete patch contract:

```text
overwrite first 5 bytes: 55 8D 6C 24 D8
write E9 rel32 to trampoline at D2Sigma+AE020
trampoline reads candidate at [ESP+28] after pushfd; pushad
trampoline replays 55 8D 6C 24 D8 after popad; popfd
trampoline jumps back to D2Sigma+AE025
```

Keep the trampoline simple: save raw candidate, timestamp, and sequence only. Do not deeply dereference `candidate` inside game code; Rust can validate safely with `ReadProcessMemory` after the hook records the pointer.

Validation should remain conservative:

```text
UnitAny.type == 4
UnitAny.class_id < 0x10000
UnitAny.mode != 3
UnitAny.data != 0
```

### Step 2: Use Freshness As The Empty-Hover Guard

The hotkey handler should accept the saved `UnitAny*` only if the last hook hit is fresh. Based on observed gaps, `500-750ms` is a safer first window than `200-300ms`, because Paragon's Hammer produced a `234ms` max gap in CE coverage.

```text
fresh = now - last_hit_tick <= 500..750ms
fresh item -> use saved pUnit
stale item -> open empty search
```

This prevents stale tooltip text or stale pointer cache values from opening the search for the previous item after the cursor leaves the item.

### Step 3: Keep Tooltip Text As Display Data, Not Identity

The UTF-16 tooltip buffer around `0x00194080..0x00194280` can still provide exact rendered tooltip text if needed. It should not be the primary identity source because it remains stale on empty hover.

Recommended usage:

```text
fresh hook pUnit -> authoritative identity
tooltip buffer   -> optional rendered text/copy support while hook state is fresh
```

Primary search query flow:

```text
fresh AE020 pUnit -> D2Injector::get_item_name(pUnit) -> strip color codes -> last non-empty item name line -> overlay event -> search_mxl_items(query)
```

### Step 4: Keep Mouse-Grid Lookup As Fallback Only

The confirmed `pInventory->pGrids` path remains useful for validation or as a fallback if the hook cannot be installed. It should not be the default identity path because it depends on screen/client geometry and multi-cell item layout.

### Optional Remaining RE

Only do more RE if broader coverage is needed before implementation:

```text
vendor item tooltips
ground item labels
other game resolutions/window modes
future D2Sigma build/version drift
```

## Current Recommendation

Do not use mouse-grid mapping as the primary identity path if the goal is exact native hovered-item targeting.

Implement `D2Sigma+AE020` as the primary native hovered-item source, guarded by timestamp freshness, with `D2Sigma+65970`, `D2Sigma+6927B`, and `D2Sigma+6973B` kept as fallback hook candidates if needed.

This matches the user's concern: the game/mod decides which item is hovered, not our coordinate math.
