# Loot P0 Hook Mask Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the P0 loot-filter failure mode where ground loot stops updating until a filter edit calls `clear_cache()`, with focus on hook-mask lifecycle and `seen_items` semantics.

**Architecture:** Keep the existing injected trampoline approach. Separate three concepts that are currently coupled: `seen_items` means “this item has been enriched/scanned”, hook-bit tracking means “remote hide/show/inspected mask bits may exist and must be cleaned”, and filter decisions mean “this item has a decision for the current filter generation”. Fix stale hook bits with a pure, testable tracker and keep runtime logging limited to actionable cleanup failures.

**Tech Stack:** Rust backend, Tauri v2, WinAPI `ReadProcessMemory`/`WriteProcessMemory`, existing `LootFilterHook` trampoline.

---

## File Structure

- Create `src-tauri/src/hook_bit_tracker.rs`: pure Rust state machine for hook-bit cleanup lifecycle; unit-tested without D2.
- Modify `src-tauri/src/main.rs`: add `mod hook_bit_tracker;`.
- Modify `src-tauri/src/notifier.rs`: replace `missed_ticks` cleanup ownership with `HookBitTracker`; keep `seen_items` focused on scan/enrichment identity.
- Modify `src-tauri/src/loot_filter_hook.rs`: keep hook mask operations correct without aggregate write telemetry.
- Update `docs/loot-performance-feedback-bugs.md`: mark the hook-bit cleanup bug as actively addressed and record the final behavior.

## Current Behavior To Preserve

- `add_inspected_unit_id()` remains necessary. It means “Rust already analyzed this item”. The trampoline hides uninspected items to prevent label flicker before Rust has applied `hide/show/default` decisions.
- A user changing loot-filter rules may reprocess visible ground loot. That behavior is acceptable.
- `seen_items` should still allow re-notification when the same physical item leaves the ground and is dropped again.

## Task 1: Add Pure Hook Bit Tracker

**Files:**
- Create: `src-tauri/src/hook_bit_tracker.rs`

- [ ] **Step 1: Create failing tests for cleanup lifecycle**

Create `src-tauri/src/hook_bit_tracker.rs` with this initial test module and a minimal empty struct so the file compiles once implementation is added:

```rust
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct HookBitTracker {
    threshold: u8,
    tracked: HashSet<u32>,
    missed: HashMap<u32, u8>,
}

impl HookBitTracker {
    pub fn new(threshold: u8) -> Self {
        Self {
            threshold,
            tracked: HashSet::new(),
            missed: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[u32]) -> HashSet<u32> {
        values.iter().copied().collect()
    }

    #[test]
    fn clears_after_threshold_even_if_scan_seen_state_would_drop_id() {
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(42);

        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
        assert_eq!(tracker.tracked_len(), 1);

        assert_eq!(tracker.plan_clears(&ids(&[])), vec![42]);
        assert_eq!(tracker.tracked_len(), 1);

        tracker.confirm_cleared(&[42]);
        assert_eq!(tracker.tracked_len(), 0);
        assert_eq!(tracker.missed_len(), 0);
    }

    #[test]
    fn retries_until_clear_is_confirmed() {
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(7);

        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
        assert_eq!(tracker.plan_clears(&ids(&[])), vec![7]);

        // Simulate clear_unit_id_bits failure: do not call confirm_cleared.
        assert_eq!(tracker.plan_clears(&ids(&[])), vec![7]);

        tracker.confirm_cleared(&[7]);
        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
    }

    #[test]
    fn reappearing_item_resets_missed_count() {
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(99);

        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
        assert_eq!(tracker.missed_len(), 1);

        assert_eq!(tracker.plan_clears(&ids(&[99])), Vec::<u32>::new());
        assert_eq!(tracker.missed_len(), 0);

        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
        assert_eq!(tracker.plan_clears(&ids(&[])), vec![99]);
    }

    #[test]
    fn clear_resets_all_state() {
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(1);
        tracker.mark_written(2);
        let _ = tracker.plan_clears(&ids(&[]));

        tracker.clear();

        assert_eq!(tracker.tracked_len(), 0);
        assert_eq!(tracker.missed_len(), 0);
        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
    }
}
```

- [ ] **Step 2: Run tests and verify they fail on missing methods**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hook_bit_tracker -- --nocapture`

Expected: FAIL with errors for missing methods `mark_written`, `plan_clears`, `confirm_cleared`, `tracked_len`, `missed_len`, and `clear`.

- [ ] **Step 3: Implement the tracker**

Replace the `impl HookBitTracker` block with:

```rust
impl HookBitTracker {
    pub fn new(threshold: u8) -> Self {
        Self {
            threshold: threshold.max(1),
            tracked: HashSet::new(),
            missed: HashMap::new(),
        }
    }

    pub fn mark_written(&mut self, unit_id: u32) {
        self.tracked.insert(unit_id);
        self.missed.remove(&unit_id);
    }

    pub fn plan_clears(&mut self, current_item_ids: &HashSet<u32>) -> Vec<u32> {
        let mut out = Vec::new();
        for &unit_id in self.tracked.iter() {
            if current_item_ids.contains(&unit_id) {
                self.missed.remove(&unit_id);
                continue;
            }

            let count = self.missed.entry(unit_id).or_insert(0);
            *count = count.saturating_add(1);
            if *count >= self.threshold {
                out.push(unit_id);
            }
        }
        out.sort_unstable();
        out
    }

    pub fn confirm_cleared(&mut self, unit_ids: &[u32]) {
        for &unit_id in unit_ids {
            self.tracked.remove(&unit_id);
            self.missed.remove(&unit_id);
        }
    }

    pub fn clear(&mut self) {
        self.tracked.clear();
        self.missed.clear();
    }

    pub fn tracked_len(&self) -> usize {
        self.tracked.len()
    }

    pub fn missed_len(&self) -> usize {
        self.missed.len()
    }
}
```

- [ ] **Step 4: Verify tracker tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hook_bit_tracker -- --nocapture`

Expected: PASS for all `hook_bit_tracker` tests.

## Task 2: Integrate HookBitTracker Into Scanner Cleanup

**Files:**
- Modify: `src-tauri/src/main.rs:3-23`
- Modify: `src-tauri/src/notifier.rs:95-134`, `src-tauri/src/notifier.rs:222-260`, `src-tauri/src/notifier.rs:557-684`

- [ ] **Step 1: Register the new module**

Add this module line in `src-tauri/src/main.rs` next to the other backend modules:

```rust
mod hook_bit_tracker;
```

- [ ] **Step 2: Import and replace scanner fields**

In `src-tauri/src/notifier.rs`, add this import near the other crate imports:

```rust
use crate::hook_bit_tracker::HookBitTracker;
```

Replace the `missed_ticks` field and comment with:

```rust
    /// Tracks remote hook-mask bits that may exist for processed items.
    /// This is separate from `seen_items`: an item can be removed from
    /// scan/enrichment identity while its remote hide/show/inspected bits
    /// still need cleanup after the missed-tick grace period.
    hook_bits: HookBitTracker,
```

Keep the existing constant:

```rust
#[cfg(target_os = "windows")]
const MISSED_TICKS_BEFORE_BIT_CLEAR: u8 = 2;
```

- [ ] **Step 3: Initialize and clear hook tracker**

In `DropScanner::new`, replace `missed_ticks: HashMap::new(),` with:

```rust
            hook_bits: HookBitTracker::new(MISSED_TICKS_BEFORE_BIT_CLEAR),
```

In `clear_cache()`, replace `self.missed_ticks.clear();` with:

```rust
        self.hook_bits.clear();
```

- [ ] **Step 4: Mark ids when hook bits may have been written**

In `tick_items()`, before the visibility match at `if self.loot_hook.is_injected()`, introduce a local flag after `let mut should_emit = true;`:

```rust
                    let mut hook_bits_may_exist = false;
```

Inside the `if self.loot_hook.is_injected()` block that applies `Show`/`Hide`, set the flag before the `match`:

```rust
                                    hook_bits_may_exist = true;
```

Inside the later `if self.loot_hook.is_injected()` block that calls `add_inspected_unit_id()`, set the flag before the write:

```rust
                        hook_bits_may_exist = true;
```

After the inspected write block, add:

```rust
                    if hook_bits_may_exist {
                        self.hook_bits.mark_written(unit_id);
                    }
```

This deliberately tracks ids even when an individual write failed; clearing a bit that was never set is harmless, and tracking avoids stale bits when only one of several hook writes succeeded.

- [ ] **Step 5: Replace missed cleanup with tracker cleanup**

Replace the current block that iterates `self.seen_items.iter()` and mutates `self.missed_ticks` with:

```rust
        // Hook-mask cleanup is intentionally independent from `seen_items`.
        // `seen_items` is pruned immediately so re-dropped items can notify,
        // but remote hook bits must survive in this tracker until they are
        // actually cleared or retried after a write failure.
        let to_clear = self.hook_bits.plan_clears(&current_item_ids);
        if !to_clear.is_empty() && self.loot_hook.is_injected() {
            match self
                .loot_hook
                .clear_unit_id_bits(&self.state.ctx, &to_clear)
            {
                Ok(()) => self.hook_bits.confirm_cleared(&to_clear),
                Err(e) => log_error(&format!(
                    "Failed to clear hook bits for {} departed items: {}",
                    to_clear.len(),
                    e
                )),
            }
        }
```

Keep the existing `self.seen_items.retain(|id| current_item_ids.contains(id));` after this block.

- [ ] **Step 6: Verify Rust tests and compilation**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hook_bit_tracker -- --nocapture`

Expected: PASS.

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: exit code 0.

## Task 3: Add Minimal Hook Lifecycle Diagnostics

**Files:**
- Modify: `src-tauri/src/hook_bit_tracker.rs`
- Modify: `src-tauri/src/notifier.rs:650-684`

- [ ] **Step 1: Add diagnostic accessors to HookBitTracker**

Add this method to `HookBitTracker`:

```rust
    pub fn overdue_len(&self) -> usize {
        self.missed
            .values()
            .filter(|&&count| count >= self.threshold)
            .count()
    }
```

Add this test to the existing test module:

```rust
    #[test]
    fn overdue_len_counts_failed_clear_retries() {
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(5);

        assert_eq!(tracker.overdue_len(), 0);
        let _ = tracker.plan_clears(&ids(&[]));
        assert_eq!(tracker.overdue_len(), 0);
        let _ = tracker.plan_clears(&ids(&[]));
        assert_eq!(tracker.overdue_len(), 1);
    }
```

- [ ] **Step 2: Run tracker tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml hook_bit_tracker -- --nocapture`

Expected: PASS.

- [ ] **Step 3: Log only anomalous cleanup retries**

In `hook_bit_tracker.rs`, add a small `HookCleanupFailureLogThrottle` state machine and tests so persistent `clear_unit_id_bits()` failures log immediately, suppress repeated failures for a fixed scanner-tick interval, then log again with the suppressed retry count.

In `notifier.rs`, store the throttle on `DropScanner`, reset it after successful cleanup/full mask clear/no pending cleanup, and call `log_error()` only when the throttle returns `Some(suppressed_count)`.

This avoids per-tick cleanup failure spam while preserving retry visibility.

- [ ] **Step 4: Verify compilation**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: exit code 0.

## Task 4: Document Final Behavior

**Files:**
- Modify: `docs/loot-performance-feedback-bugs.md`

- [ ] **Step 1: Update the P0 hook-bit section**

In `docs/loot-performance-feedback-bugs.md`, update `P0: Departed Item Hook Bits May Never Be Cleared` with:

```markdown
Implementation status:

- Hook-bit cleanup is now tracked independently from `seen_items`.
- Missing ids remain in the hook cleanup lifecycle until `clear_unit_id_bits()` succeeds.
- `seen_items` can still be pruned immediately to preserve re-drop notification behavior.
```

- [ ] **Step 2: Add a short note about `inspected`**

Add this paragraph to the same section:

```markdown
`inspected` is not a notification cache. It is the trampoline's safety gate: uninspected items are hidden until Rust has applied a decision, preventing a fresh hidden drop from flashing through MXL's original label logic before the scanner catches up.
```

- [ ] **Step 3: Verify docs diff**

Run: `git diff -- docs/loot-performance-feedback-bugs.md docs/superpowers/plans/2026-05-11-loot-p0-hook-mask-fixes.md`

Expected: diff contains only documentation changes for this task.

## Final Verification

Code review follow-ups included:

- [ ] Protect lower-16 collisions by avoiding departed-id batch clears while a live item shares that mask index.
- [ ] Clear stale opposing show/hide bits when applying a current item's visibility decision.
- [ ] Preserve hook-bit tracker state across partial `clear_cache()` mask-clear failures.
- [ ] Throttle repeated cleanup failure logs.

- [ ] Run: `cargo test --manifest-path src-tauri/Cargo.toml hook_bit_tracker -- --nocapture`

Expected: PASS.

- [ ] Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: exit code 0.

- [ ] Manual smoke test in D2/MXL:

1. Start app and attach to D2.
2. Enter an area with visible ground drops.
3. Confirm hidden trash does not flash before scan decisions.
4. Pick up or move away from items so they leave the scan.
5. Confirm departed items do not require filter edits to restore hidden/shown label behavior.
6. Leave a long session running and confirm filter edits are no longer needed to restore hidden/shown label behavior.

## Commit Plan

Commit after Task 2 if tests and `cargo check` pass:

```bash
git add src-tauri/src/main.rs src-tauri/src/notifier.rs src-tauri/src/hook_bit_tracker.rs
git commit -m "fix(loot-filter): keep hook bit cleanup independent from seen items"
```

Commit docs after Task 4:

```bash
git add docs/loot-performance-feedback-bugs.md docs/superpowers/plans/2026-05-11-loot-p0-hook-mask-fixes.md
git commit -m "docs: plan loot filter p0 hook mask fixes"
```
