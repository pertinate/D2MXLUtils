# MXL Item Search Targeted Lookup RE Findings

Date: 2026-05-15
Updated: 2026-05-16

## Summary

The known 1.13c hovered-item global pointer candidate does not work in the tested Median XL build. A later tooltip RE session found a better native source of truth: `D2Sigma.dll+AE020`, where `[ESP+04]` is the current tooltip item `UnitAny*` on function entry.

Targeted item search should use the `D2Sigma+AE020` hook plus timestamp freshness as the primary hovered-item source. Player inventory-grid lookup from mouse position remains a fallback/stale guard only.

Confirmed supported targeted-search areas:

- Equipment/body slots
- Inventory
- Cube
- Stash

Not confirmed:

- Vendor items. Vendor storage is not part of the local player's `pInventory->pGrids` path and needs separate RE.
- Ground item labels. Ground targeted search is intentionally out of scope.

## Failed Candidates

`D2Client.dll + 0x11BC38`, commonly listed as `D2CLIENT_SelectedInvItem` / `D2CLIENT_HoverItem`, stayed zero while hovering inventory, equipment, cube, and stash items:

```text
[D2Client.dll+11BC38] = 00000000
```

Breakpoints on the current injected helper targets did not fire while the native UI tooltip was visible:

```text
D2Client.dll+914F0  ; GetItemName helper target used by D2Injector::get_item_name
D2Client.dll+560B0  ; GetItemStat helper target used by D2Injector::get_item_stats
```

Call-site scan found callers to both functions, but none of those call sites were hit by native UI item hover in this build.

## Confirmed Native Tooltip Hook

Primary hook candidate:

```text
D2Sigma.dll+AE020
[ESP+04] = current tooltip item UnitAny*
```

Patch contract:

```asm
D2Sigma.dll+AE020  55                 push ebp
D2Sigma.dll+AE021  8D 6C 24 D8        lea ebp,[esp-28]
D2Sigma.dll+AE025  83 EC 28           sub esp,28
```

```text
patch address: D2Sigma.dll+AE020
patch size:    5 bytes
saved bytes:   55 8D 6C 24 D8
resume at:     D2Sigma.dll+AE025
```

If trampoline code starts with `pushfd; pushad`, read the original `[ESP+04]` item argument at `[ESP+28]`. The trampoline should only save raw `pUnit + timestamp/sequence`; Rust should do the safe validation via `ReadProcessMemory`.

Final observed session address:

```text
D2Sigma.dll base = 68AB0000
D2Sigma.dll+AE020 = 68B5E020
```

Validation shape:

```text
UnitAny + 0x00 = 4        ; item
UnitAny + 0x04 < 0x10000 ; plausible class id
UnitAny + 0x10 != 3      ; reject ground mode for targeted search
UnitAny + 0x14 != 0      ; ItemData*
```

Coverage observed with `docs/ce-scripts/mxl-tooltip-ae020-coverage.lua`:

```text
Paragon's Hammer: maxGap=234ms
Horadric Cube:    maxGap=31-47ms
Amulets:          maxGap=16-32ms
Shamanka:         maxGap=32ms
Catalyst:         maxGap=31-46ms
Hunter's Bow:     maxGap=16ms
empty hover:      no hits
```

Implementation should save `pUnit + timestamp/sequence` from the hook and accept it on hotkey only while fresh, initially within about `500-750ms`. Accepted fresh `pUnit` should be converted to a search query through the existing `D2Injector::get_item_name(pUnit)` path, then stripped of Diablo color codes and reduced to the last non-empty item-name line. The tooltip text buffer is not the primary search query source.

Fallback hook candidates if `AE020` proves version-specific:

```text
D2Sigma.dll+65970 -> [ESP+0C]
D2Sigma.dll+6927B -> ESI
D2Sigma.dll+6973B -> ESI
```

No stable directly pollable hover field was found. The transient pointer copies at `001940DC`, `00194468`, `001944B0`, and `001944EC` remain stale on empty hover and must not be used directly as production state.

## Confirmed Fallback Memory Path

The local player inventory chain is readable:

```text
D2Client.dll+11BBFC -> UnitAny* player
UnitAny + 0x60      -> Inventory*
Inventory + 0x0C    -> UnitAny* first item
Inventory + 0x14    -> D2InventoryGridStrc* grids
grid + 0x0C         -> UnitAny** ppItems
```

Useful item fields:

```text
UnitAny + 0x00       unit type, expected 4 for item
UnitAny + 0x0C       unit id
UnitAny + 0x10       mode, reject 3 for ground items
UnitAny + 0x14       ItemData*
UnitAny + 0x60       Inventory* for container-like item units
ItemData + 0x44      body location byte
ItemData + 0x45      item location byte
ItemData + 0x5C      owner Inventory*
ItemData + 0x64      next inventory item pointer
ItemData + 0x68      game location byte
ItemData + 0x69      node page byte
```

## Confirmed Grids

`Inventory + 0x14` points to an array of `D2InventoryGridStrc` records. Existing project offsets already define:

```text
D2InventoryGridStrc + 0x0C = ppItems
D2InventoryGridStrc size   = 0x10
```

Confirmed local player grids:

```text
grid[00] equipment/body slots
grid[02] inventory, 15x10 cells, ItemData.gameLoc == 3
grid[05] cube,      15x10 cells, ItemData.gameLoc == 6
grid[06] stash,     14x14 cells, ItemData.gameLoc == 7
```

Equipment/body slot indices follow `D2C_PlayerBodyLocs`:

```text
1  head
2  neck
3  torso
4  right arm
5  left arm
6  right ring
7  left ring
8  belt
9  feet
10 gloves
```

## Screen-Space Calibration Evidence For Fallback

These coordinates were captured through Cheat Engine's `getMousePos()` with Diablo II running in a `1400x1080` window. CE failed to convert to client coordinates (`client=nil`), so these are evidence for geometry and relative spacing, not portable constants to hardcode directly.

All container cells were observed to use approximately `40x40` screen pixels in this setup.

Container samples:

```text
inventory first cell center: 1349,691
inventory last cell center:  1910,1054
inventory col 6 row 5:      1551,851
inventory col 13 row 5:     1834,852
inventory col 2 row 9:      1393,1006

stash first cell center:     1490,454
stash last cell center:      2012,972

cube first cell center:      1474,535
cube last cell center:       2031,897
```

Equipment slot center samples:

```text
head:       2193,367
neck:       2287,440
torso:      2198,495
right arm:  2370,446
left arm:   2028,435
right ring: 2290,564
left ring:  2117,571
belt:       2195,606
feet:       2367,579
gloves:     2025,582
```

Runtime implementation should read the foreground cursor position through WinAPI, call `ScreenToClient` for the Diablo II window, then compare client coordinates against calibrated UI rectangles. Do not hardcode the screen-space coordinates above without converting them to client-space for the active D2 window.

## Cheat Engine Helpers

The original grid/mouse RE helper script lives at:

```text
docs/ce-scripts/mxl-hovered-item-re.lua
```

Useful functions:

```text
mxl_dump_player_inventory()
mxl_dump_item_fields(80)
mxl_dump_inventory_grids(7)
mxl_start_mouse_trace(30, 250)
mxl_stop_mouse_trace()
stop_mxl_hover_re()
```

The final tooltip-hook scripts live at:

```text
docs/ce-scripts/mxl-tooltip-cleanup.lua
docs/ce-scripts/mxl-tooltip-ae020-coverage.lua
docs/ce-scripts/mxl-tooltip-candidate-hook-scout.lua
docs/ce-scripts/mxl-tooltip-context-write-scout.lua
docs/ce-scripts/mxl-tooltip-hooksite-scout.lua
docs/ce-scripts/mxl-tooltip-flag-scout.lua
docs/ce-scripts/mxl-tooltip-polling-scout.lua
docs/ce-scripts/mxl-tooltip-freshness-scout.lua
docs/ce-scripts/mxl-tooltip-multi-hook-scout.lua
```

The old helper also includes breakpoint setup for the failed `GetItemName` / `GetItemStat` hover-path investigation. The current primary implementation path is the `D2Sigma+AE020` tooltip hook; grid lookup is fallback only.
