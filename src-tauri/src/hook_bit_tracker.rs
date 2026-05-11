use std::collections::{HashMap, HashSet};

use crate::loot_filter_hook::VisibilityMaskOp;

const MASK_INDEX_BITS: u32 = 0xFFFF;

#[derive(Debug, Clone)]
pub struct HookBitTracker {
    threshold: u8,
    tracked: HashSet<u32>,
    missed: HashMap<u32, u8>,
}

#[derive(Debug, Clone)]
pub struct HookCleanupFailureLogThrottle {
    suppressed_failure_ticks: u32,
    ticks_since_log: Option<u32>,
    suppressed_failures: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PendingVisibilityMaskOps {
    pending: HashMap<u32, Vec<VisibilityMaskOp>>,
}

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
        let protected_mask_indexes: HashSet<u32> = current_item_ids
            .iter()
            .map(|unit_id| unit_id & MASK_INDEX_BITS)
            .collect();

        for &unit_id in self.tracked.iter() {
            if current_item_ids.contains(&unit_id) {
                self.missed.remove(&unit_id);
                continue;
            }

            if protected_mask_indexes.contains(&(unit_id & MASK_INDEX_BITS)) {
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

    #[cfg(test)]
    pub fn missed_len(&self) -> usize {
        self.missed.len()
    }

    pub fn overdue_len(&self) -> usize {
        self.missed
            .values()
            .filter(|&&count| count >= self.threshold)
            .count()
    }

    pub fn departed_mask_collisions(
        &self,
        unit_id: u32,
        current_item_ids: &HashSet<u32>,
    ) -> Vec<u32> {
        if !current_item_ids.contains(&unit_id) {
            return Vec::new();
        }

        let mask_index = unit_id & MASK_INDEX_BITS;
        let mut out: Vec<u32> = self
            .tracked
            .iter()
            .copied()
            .filter(|&tracked_id| {
                tracked_id != unit_id
                    && !current_item_ids.contains(&tracked_id)
                    && (tracked_id & MASK_INDEX_BITS) == mask_index
            })
            .collect();
        out.sort_unstable();
        out
    }
}

impl HookCleanupFailureLogThrottle {
    pub fn new(suppressed_failure_ticks: u32) -> Self {
        Self {
            suppressed_failure_ticks: suppressed_failure_ticks.max(1),
            ticks_since_log: None,
            suppressed_failures: 0,
        }
    }

    pub fn record_failure(&mut self) -> Option<u64> {
        match self.ticks_since_log {
            None => {
                self.ticks_since_log = Some(0);
                Some(0)
            }
            Some(ticks) if ticks >= self.suppressed_failure_ticks => {
                let suppressed = self.suppressed_failures;
                self.ticks_since_log = Some(0);
                self.suppressed_failures = 0;
                Some(suppressed)
            }
            Some(ticks) => {
                self.ticks_since_log = Some(ticks.saturating_add(1));
                self.suppressed_failures = self.suppressed_failures.saturating_add(1);
                None
            }
        }
    }

    pub fn reset(&mut self) {
        self.ticks_since_log = None;
        self.suppressed_failures = 0;
    }
}

impl PendingVisibilityMaskOps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&mut self, unit_id: u32) -> Vec<VisibilityMaskOp> {
        self.pending.remove(&unit_id).unwrap_or_default()
    }

    pub fn record_failed(&mut self, unit_id: u32, failed_ops: Vec<VisibilityMaskOp>) {
        if failed_ops.is_empty() {
            self.pending.remove(&unit_id);
        } else {
            self.pending.insert(unit_id, failed_ops);
        }
    }

    pub fn retain_current(&mut self, current_item_ids: &HashSet<u32>) {
        self.pending
            .retain(|unit_id, _| current_item_ids.contains(unit_id));
    }

    pub fn clear(&mut self) {
        self.pending.clear();
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
    fn current_item_with_same_mask_index_blocks_clear() {
        let old_unit_id = 9;
        let colliding_live_unit_id = old_unit_id + 0x1_0000;
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(old_unit_id);

        assert_eq!(
            tracker.plan_clears(&ids(&[colliding_live_unit_id])),
            Vec::<u32>::new()
        );
        assert_eq!(
            tracker.plan_clears(&ids(&[colliding_live_unit_id])),
            Vec::<u32>::new()
        );
        assert_eq!(tracker.missed_len(), 0);

        assert_eq!(tracker.plan_clears(&ids(&[])), Vec::<u32>::new());
        assert_eq!(tracker.plan_clears(&ids(&[])), vec![old_unit_id]);
    }

    #[test]
    fn departed_mask_collision_requires_current_item_reset() {
        let old_unit_id = 9;
        let colliding_live_unit_id = old_unit_id + 0x1_0000;
        let current = ids(&[colliding_live_unit_id]);
        let mut tracker = HookBitTracker::new(2);
        tracker.mark_written(old_unit_id);

        assert_eq!(
            tracker.departed_mask_collisions(colliding_live_unit_id, &current),
            vec![old_unit_id]
        );
        assert_eq!(
            tracker.departed_mask_collisions(old_unit_id, &current),
            Vec::<u32>::new()
        );
        assert_eq!(
            tracker.departed_mask_collisions(10, &current),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn pending_visibility_ops_retry_only_failed_ops_until_success() {
        let mut pending = PendingVisibilityMaskOps::new();

        pending.record_failed(
            55,
            vec![VisibilityMaskOp::SetHide, VisibilityMaskOp::ClearShow],
        );
        assert_eq!(
            pending.take(55),
            vec![VisibilityMaskOp::SetHide, VisibilityMaskOp::ClearShow]
        );

        pending.record_failed(55, vec![VisibilityMaskOp::ClearShow]);
        assert_eq!(pending.take(55), vec![VisibilityMaskOp::ClearShow]);

        pending.record_failed(55, Vec::new());
        assert_eq!(pending.take(55), Vec::<VisibilityMaskOp>::new());
    }

    #[test]
    fn cleanup_failure_throttle_suppresses_and_resets() {
        let mut throttle = HookCleanupFailureLogThrottle::new(2);

        assert_eq!(throttle.record_failure(), Some(0));
        assert_eq!(throttle.record_failure(), None);
        assert_eq!(throttle.record_failure(), None);
        assert_eq!(throttle.record_failure(), Some(2));
        assert_eq!(throttle.record_failure(), None);

        throttle.reset();

        assert_eq!(throttle.record_failure(), Some(0));
    }
}
