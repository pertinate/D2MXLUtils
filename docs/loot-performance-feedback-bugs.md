# Loot/Marker Performance Feedback Bugs

This document breaks recent user feedback into concrete bugs and investigation tracks. The reports cluster around delayed loot notifications, inconsistent ground-label visibility, missed or unstable minimap markers, and duplicate sounds after filter changes.

## Feedback Summary

Users reported the following symptoms:

- Minimap markers sometimes fail for drops, especially suspected off-screen drops.
- After 30+ minutes the app becomes laggy/buggy and randomly stops showing loot.
- Changing the notification/filter settings can make loot notifications or labels start working again.
- Large filters can cause 3-10 second delays, and over time the system can stop working entirely.
- Items visible on the ground can fail to produce notifications, even for important drops like Sacred Uniques.
- Sounds can play multiple times for the same drop, including delayed repeat playback.
- Minimap icons for altars, chests, quest entrances, and waypoints can blink or disappear after filter changes.

## Current Architecture Notes

Markers are no longer in the same synchronous path as notification emission. `tick_items()` emits `item-drop` before marker work runs, caches runtime filter decisions for scanned items, and marker scanning is handled by a separate `marker-scanner` thread.

Relevant paths:

- Item scan and notifications: `src-tauri/src/notifier.rs`, `src-tauri/src/main.rs`
- Filter matching: `src-tauri/src/rules/matching.rs`, `src-tauri/src/rules/mod.rs`
- Marker scanning and automap writes: `src-tauri/src/marker_scanner.rs`, `src-tauri/src/map_marker.rs`
- Hook masks for show/hide/inspected labels: `src-tauri/src/loot_filter_hook.rs`
- Overlay notification and sound playback: `src/views/OverlayWindow.svelte`, `src/lib/sound-player.ts`

Even though marker work is split out, it can still consume CPU through BFS, contend on the shared injector mutex, and mutate the live automap object tree. After `3394422`, marker scanning no longer repeats heavy filter matching for marker candidates; it consumes cached item-scan decisions keyed by filter generation.

## Bugs

### P0: Large Filters Cause Multi-Second Delays

Status after `3394422`:

- Likely fixed for the two main identified hotspots. Runtime regex compilation was moved out of the per-item match path via `FilterConfig::prepare_for_matching()`, and marker scanning now reads `recent_filter_decisions` instead of running `filter.decide()` again.
- Also addressed the no-map marker path: marker scanning clears existing markers once and returns before BFS when the loaded filter has no `map` rules.
- Not fully proven by diagnostics yet. Reverse-order last-match rule scanning remains `O(rules)` per scanned item, marker BFS still runs every 30 ms when any `map` rule exists, and tick-duration/decision-duration percentiles still need measurement on large real filters.

Symptoms:

- With big filters, users see 3-10 second delays before loot notifications appear.
- Some users report the system eventually stops working during long sessions.

Original evidence in code before `3394422`:

- `src-tauri/src/rules/matching.rs:111-116` compiles `Regex::new()` during every pattern match.
- `src-tauri/src/rules/mod.rs:278-279` scans rules in reverse order until the winning rule is found.
- `src-tauri/src/marker_scanner.rs:103-104` repeats `filter.decide()` in the marker thread for marker candidates.

Root-cause hypothesis:

- Runtime regex compilation is the main hotspot for large filters: `items x rules x patterns` can explode quickly.
- Marker scanning doubles part of the matching cost for items that also need map-marker evaluation.

Fix status:

- Done in `3394422`: precompile rule patterns when loading the runtime filter.
- Done in `3394422`: add a filter decision cache keyed by stable item identity plus filter generation.
- Done in `3394422`: short-circuit marker scanning when the filter has no `map` rules, clearing existing app markers once.
- Throttle marker BFS or run it adaptively instead of every 30 ms.

Diagnostics to add:

- Per-item `filter.decide()` duration.
- Rule count, pattern count, regex compile count.
- Marker-thread `filter.decide()` duration.
- Tick duration percentiles for `tick_items()` and marker BFS.

### P0: Items Can Become Seen Before They Notify

Symptoms:

- Loot is visible on the ground but never notifies.
- Changing the notification filter can make old/current drops notify suddenly.

Evidence in code:

- `src-tauri/src/notifier.rs:717-720` skips items already present in `seen_items`.
- `src-tauri/src/notifier.rs:777-778` inserts the item into `seen_items` after scan/enrichment.
- `src-tauri/src/notifier.rs:515-588` applies filter decisions after scan/enrichment.

Root-cause hypothesis:

- An item can be marked as seen during a moment when the filter config is stale, missing, invalid, or has no matching `notify` rule. Later filter updates do not reconsider that item until a full cache clear happens.

Proposed fixes:

- Split item identity tracking from notification/decision tracking.
- Track whether filter decision, hook bits, map marker, and notification were applied for each item.
- Re-evaluate already-seen items when filter generation changes, without treating them as fresh drops for notification sounds.

Diagnostics to add:

- Per item: `unit_id`, `seed`, filter generation, whether it was already seen, whether notification was emitted.
- Count of items skipped due to `seen_items` while filter generation changed.
- Count of items seen with `notification = None`.

### P0: Loot Stops Updating Until Filter Cache Is Cleared

Status after `ff4e2b5`:

- Partially/directly addressed for the stale hook-mask path. The commit makes hook-mask cleanup independent from `seen_items`, retries failed hook-bit cleanup, preserves cleanup state across partial mask-clear failures, and clears stale opposing show/hide bits for current decisions.
- Not fully closed as a whole bug class. Other possible causes listed below, especially stale `seen_items` notification/decision state and stale `recent_events`, still need separate investigation.

Status after `3394422`:

- Partially reduced risk from large-filter CPU stalls: expensive runtime regex compilation is removed from the hot path, and marker scanning no longer duplicates full filter matching.
- Not fixed as a stale-state bug class. `seen_items`, `recent_events`, duplicate overlay events/sounds, and filter-change reprocessing semantics still need separate investigation.

Symptoms:

- Loot display/notifications can stop working during a session.
- Changing notification/filter settings makes loot start showing again.
- The problem is not that existing loot replays after a filter edit. Re-evaluating visible ground loot after a rule change is acceptable behavior. The bug is that changing the filter appears to be the only workaround for a stuck scanner/filter/hook state.

Evidence in code:

- `src-tauri/src/notifier.rs:258-260` calls `clear_cache()` on filter config changes.
- `src-tauri/src/notifier.rs:346-361` clears `seen_items`, `recent_events`, and hook masks.
- `src/views/OverlayWindow.svelte:105-109` plays a sound for every `item-drop` event.
- `src/lib/sound-player.ts:74-78` allows overlapping playback and has no item-level dedupe.

Root-cause hypothesis:

- `clear_cache()` resets `seen_items`, `recent_events`, `missed_ticks`, `seen_goblins`, and hook masks. If the app gets stuck because an item was marked seen without a valid decision, because hook masks contain stale state, or because `recent_events` no longer reflects the ground state, changing the filter temporarily fixes the session by forcing a full re-evaluation. The root bug is the stale/stuck state before the cache clear, not the re-evaluation itself.

Proposed fixes:

- Identify which cache/state becomes stale before the filter edit: `seen_items`, `recent_events`, hook masks, or filter config generation.
- Re-evaluate decisions separately from fresh-drop detection so already-seen items can recover without requiring a full cache clear.
- Keep explicit state for `decision_applied`, `hook_bits_applied`, `notification_emitted`, and `marker_applied` per item.
- Preserve the intentional behavior that a user changing rules can reprocess visible ground loot according to the new rules.

Diagnostics to add:

- Log every filter generation bump with current `seen_items.len()` and `recent_events.len()`.
- Before `clear_cache()`, log whether stuck items were present in `seen_items`, `recent_events`, and hook masks.
- Log items skipped because they were already in `seen_items` while no notification/decision had been applied.
- Log hook mask state for items that become visible only after the filter edit.

### P1: Off-Screen Drops Can Miss Markers and Notifications

Status after `3394422`:

- Not fixed. Marker placement no longer depends on `recent_events` directly, but it still depends on the item scanner having enriched the item and cached a filter decision in `recent_filter_decisions`.
- BFS-only discoveries are still not promoted into item enrichment candidates, so off-screen items that only marker BFS sees can still miss notifications and markers.

Symptoms:

- Users suspect off-screen drops are the ones that fail to notify or receive markers.
- Some drops are neither notified nor marked on the minimap.

Evidence in code after `3394422`:

- `src-tauri/src/marker_scanner.rs:66` discovers item positions through BFS.
- `src-tauri/src/marker_scanner.rs:130-142` only places a marker if the item has a current-generation cached decision in `recent_filter_decisions`.
- `src-tauri/src/notifier.rs:684-689` prunes `recent_events` to items visible in the current item scan.

Root-cause hypothesis:

- The marker BFS can see item positions that the item scanner has not enriched. Since markers now depend on `recent_filter_decisions`, BFS-only items are still ignored. If the item scanner also does not see the item through `pPaths`, no notification is emitted either.

Proposed fixes:

- Treat BFS-found unknown items as discovery candidates and enqueue them for enrichment in the item scanner.
- Keep `recent_events` by TTL/stable identity instead of pruning only by current `pPaths` visibility.
- Add instrumentation to compare BFS item ids with item-scan item ids.

Diagnostics to add:

- BFS item count.
- `BFS ids missing in recent_events` count.
- `pPaths current_item_ids` count.
- Distance/depth at which missed drops are found.

### P1: Automap Icons Blink or Disappear

Status after `3394422`:

- Slightly reduced churn for filters with no `map` rules: marker scanning clears app markers once and skips BFS/reconciliation while no map rules exist.
- Not fixed for active marker rules. The automap chain can still be detached/attached during marker input changes, generation transitions, tamper recovery, or empty marker snapshots.

Symptoms:

- Minimap icons for altars, quest markers, chests, entrances, and waypoints blink or disappear.
- The issue can happen after changing a few lines in the filter.

Evidence in code:

- `src-tauri/src/map_marker.rs:156-164` detaches and attaches the marker chain during reconciliation.
- `src-tauri/src/map_marker.rs:197-235` prepends allocated marker cells into `AutomapLayer.pObjects`.
- `src-tauri/src/map_marker.rs:254-255` drops persistent markers immediately when BFS sees the item but current filter matching does not produce a marker.

Root-cause hypothesis:

- The app mutates the live automap object tree while the game/MXL may also update native icons. Filter changes can create rapid detach/attach churn and temporary empty snapshots, causing marker chains and native icons to blink or disappear.

Proposed fixes:

- Avoid rebuilding the automap chain when marker input is temporarily empty due to filter generation changes.
- Freeze marker reconciliation for 1-2 item ticks after filter config changes.
- Reduce automap tree churn by hashing inputs and only writing when stable.
- Add stronger tamper and stale-root diagnostics before changing the splice logic.

Diagnostics to add:

- Log each automap detach/attach with layer, old root, current root, marker count, and filter generation.
- Count marker chain rebuilds per second.
- Log tamper-check failures and orphaned chain resets.

### P0: Departed Item Hook Bits May Never Be Cleared

Status after `ff4e2b5`:

- Likely fixed directly. Departed ids now remain in a dedicated hook-bit cleanup lifecycle until `clear_unit_id_bits()` succeeds, instead of depending on `seen_items` long enough to reach the missed-tick threshold.
- The fix also handles lower-16 mask-index collisions more conservatively: departed ids are not batch-cleared while a live item shares the same mask index, and fresh colliding items reset stale bits before being marked inspected.

Symptoms:

- After 30+ minutes, loot labels randomly stop showing.
- Changing the filter temporarily fixes label visibility because `clear_cache()` clears all hook masks.
- The problem can look like the scanner/filter stopped working even though the actual stale state is lower-level in the injected hook masks.

Evidence in code:

- `src-tauri/src/notifier.rs:118-122` documents `missed_ticks` as the grace period before clearing hook-mask bits.
- `src-tauri/src/notifier.rs:134` sets `MISSED_TICKS_BEFORE_BIT_CLEAR` to `2`.
- `src-tauri/src/notifier.rs:650-684` increments missed counters and only calls `clear_unit_id_bits()` when a missing item reaches the threshold.
- `src-tauri/src/notifier.rs:684` immediately removes any missing id from `seen_items` with `self.seen_items.retain(|id| current_item_ids.contains(id))`.
- `src-tauri/src/loot_filter_hook.rs:474-499` has the batch clear primitive, but it only runs for ids that remain tracked long enough to reach the missed-tick threshold.

Root-cause hypothesis:

- On the first tick where an item is missing, `missed_ticks[id]` becomes `1`, which is below the threshold. At the end of that same tick, the item is removed from `seen_items`. On the next tick, that id is no longer iterated, so it can never reach `MISSED_TICKS_BEFORE_BIT_CLEAR = 2`. Its `hide`, `show`, and `inspected` bits can remain set until a full `clear_cache()`.
- Because hook masks are indexed by `unit_id & 0xFFFF`, stale bits can affect a later item whose lower 16 bits collide with the departed item.

Why this is low-level:

- Hiding itself is not done by running remote code per item. The scanner writes bits into remote masks with `ReadProcessMemory`/`WriteProcessMemory`, and the injected trampoline checks those masks in the game thread.
- A stale mask bit can bypass the higher-level filter logic entirely: the trampoline may return hide/show based on old mask state before Rust gets a chance to apply a fresh decision for the new item.

Proposed fixes:

- Do not remove an id from the mask-tracking lifecycle until its hook bits have been cleared or deliberately retained.
- Track hook-bit state separately from `seen_items`; `seen_items` can represent notification/enrichment identity, while a `tracked_mask_bits` set represents ids that still need cleanup.
- Alternatively, move the retain after the missed threshold lifecycle: keep missing ids in the cleanup path until `clear_unit_id_bits()` succeeds.
- Log and retry `clear_unit_id_bits()` failures instead of silently losing cleanup state.

Diagnostics to add:

- Count ids that enter `missed_ticks` but are removed from `seen_items` before reaching the clear threshold.
- Log hook bit clear batches with ids, lower-16 mask indexes, and success/failure.
- Add a debug counter for stale lower-16 collisions: different full `unit_id` values sharing the same mask bit while old bits are still set.

Implementation status:

- Hook-bit cleanup is now tracked independently from `seen_items`.
- Missing ids remain in the hook cleanup lifecycle until `clear_unit_id_bits()` succeeds.
- `seen_items` can still be pruned immediately to preserve re-drop notification behavior.
- Stale ids are not batch-cleared while any current item shares the same lower-16 mask index.
- Current visibility decisions also clear opposing show/hide bits for that mask index, so a stale colliding `show` bit cannot override a fresh `hide` decision and stale visibility bits are removed for `default` decisions.
- `clear_cache()` only drops local hook-bit tracking after full remote mask clears succeed.
- Repeated `clear_unit_id_bits()` failures are retried without logging every scanner tick.

`inspected` is not a notification cache. It is the trampoline's safety gate: uninspected items are hidden until Rust has applied a decision, preventing a fresh hidden drop from flashing through MXL's original label logic before the scanner catches up.

### P1: Long Sessions Can Accumulate Wrong Hook Mask State

Status after `ff4e2b5`:

- Partially/directly addressed for the concrete stale departed-bit path described in the P0 hook-bit bug above.
- Not fully closed. The broader limitation remains: hook masks are still indexed by `unit_id & 0xFFFF`, so two simultaneously live items with the same lower-16 mask index cannot be represented independently by the current mask design.

Symptoms:

- After 30+ minutes, loot labels randomly stop showing.
- Filter changes can temporarily fix label visibility.

Evidence in code:

- `src-tauri/src/loot_filter_hook.rs:47-49` indexes hook masks by `unit_id & 0xFFFF`.
- `src-tauri/src/notifier.rs:650-684` attempts to clear hook bits for departed items after missed ticks.
- `src-tauri/src/notifier.rs:346-361` full cache clears also clear hook masks, which can explain why filter changes appear to fix stale visibility.

Root-cause hypothesis:

- A stale bit or lower-16-bit unit id collision can cause a later item to inherit an old hide/show/inspected decision.
- The concrete cleanup bug above is one likely path to stale bits. This section covers the broader class of hook-mask identity/collision issues.

Proposed fixes:

- Add collision diagnostics for different full `unit_id` values sharing the same lower 16 bits.
- Revisit mask identity, possibly adding generation-based validation or more conservative clearing on area/filter transitions.
- Ensure departed-item cleanup cannot be skipped indefinitely when item scanning misses rooms or units.

Diagnostics to add:

- Log mask index collisions.
- Log hook bit clear batches and failures.
- Log `unit_id`, `unit_id & 0xFFFF`, visibility decision, and inspected bit writes for anomalous items.

### P2: Item Scanner Has Fewer Safety Caps Than Marker BFS

Symptoms:

- Rare stalls or missed items on corrupted/transient room/unit lists could amplify lag.

Evidence in code:

- `src-tauri/src/notifier.rs:460-473` reads `i_paths` from game memory and iterates without a sanity cap.
- `src-tauri/src/notifier.rs:490-647` walks a unit linked list with no explicit iteration cap.
- `src-tauri/src/map_marker.rs:321` and `src-tauri/src/map_marker.rs:365` show that BFS already uses caps for similar memory walks.

Root-cause hypothesis:

- If transient game memory produces an unexpectedly large path count or cyclic/bad unit list, item scanning can stall or break out early in ways that cause missed/repeated processing.

Proposed fixes:

- Add caps for `i_paths` and per-room unit iteration in `tick_items()`.
- Log when caps are hit.
- Prefer continuing past bad unit reads where safe instead of breaking the entire room scan.

Diagnostics to add:

- `i_paths` values per tick.
- Units visited per room.
- Read failure counts and locations.
- Cap-hit counters.

## Recommended Work Order

1. Fix departed-item hook bit cleanup so stale hide/show/inspected bits cannot survive until `clear_cache()`.
2. Fix large-filter matching cost: precompile patterns, add decision cache, and skip marker BFS when no rules need it.
3. Fix `seen_items` semantics so filter changes re-evaluate decisions without depending on a full cache reset.
4. Add diagnostics for stale seen items, hook mask cleanup, and filter generation changes.
5. Investigate off-screen discovery by comparing BFS ids with item-scan ids.
6. Stabilize automap marker reconciliation and reduce detach/attach churn.
7. Add hook mask collision/stale-state diagnostics for long-session reports.

## Minimum Diagnostic Mode

Add a temporary or user-toggleable diagnostic mode that logs aggregated counters every few seconds instead of per-item spam by default.

Useful metrics:

- `tick_items.duration_ms`
- `tick_items.items_current`
- `tick_items.new_items`
- `tick_items.skipped_seen`
- `tick_items.events_emitted`
- `filter.rules.len`
- `filter.decide.total_ms`
- `filter.regex_compile_count` before precompile fix
- `recent_events.len`
- `seen_items.len`
- `marker.bfs.duration_ms`
- `marker.bfs.items_found`
- `marker.bfs.missing_recent_events`
- `marker.rebuilds_per_minute`
- duplicate `item-drop` count by `seed`
- duplicate sound count by `seed` and slot
- hook mask lower-16 collision count
- ids removed from `seen_items` before hook bits are cleared
- hook bit clear batch count, size, and failure count
