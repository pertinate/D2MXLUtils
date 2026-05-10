//! v1.23 → v1.24: top-level `notificationX/Y` (percent) moved into
//! `widget_positions["notifications"]`. Done as part of the unified
//! overlay-widget-repositioning module — see
//! docs/superpowers/specs/2026-05-09-overlay-widget-repositioning-design.md.

use serde::Deserialize;
use serde_json::Value;

use crate::settings::{AppSettings, WidgetPosition};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")] // matches AppSettings's wire format
struct LegacyKeys {
    #[serde(default)]
    notification_x: Option<f64>,
    #[serde(default)]
    notification_y: Option<f64>,
}

pub fn apply(raw: &Value, s: &mut AppSettings) -> bool {
    if s.widget_positions.contains_key("notifications") {
        return false;
    }
    let legacy: LegacyKeys = serde_json::from_value(raw.clone()).unwrap_or_default();
    // Skip when neither legacy key was present (fresh install): the
    // helper's spec default kicks in and we avoid writing a noisy
    // pre-populated entry to settings.
    if legacy.notification_x.is_none() && legacy.notification_y.is_none() {
        return false;
    }
    let x = legacy.notification_x.unwrap_or(1.0);
    let y = legacy.notification_y.unwrap_or(1.0);
    s.widget_positions.insert(
        "notifications".into(),
        WidgetPosition { x, y },
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_settings() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn migrates_when_legacy_keys_present() {
        let raw = json!({ "notificationX": 42.5, "notificationY": 17.0 });
        let mut s = fresh_settings();

        let changed = apply(&raw, &mut s);

        assert!(changed, "should report changed=true");
        assert_eq!(
            s.widget_positions.get("notifications"),
            Some(&WidgetPosition { x: 42.5, y: 17.0 }),
        );
    }

    #[test]
    fn idempotent_when_already_migrated() {
        let raw = json!({ "notificationX": 99.0, "notificationY": 99.0 });
        let mut s = fresh_settings();
        s.widget_positions
            .insert("notifications".into(), WidgetPosition { x: 5.0, y: 7.0 });

        let changed = apply(&raw, &mut s);

        assert!(!changed, "should report changed=false on second run");
        assert_eq!(
            s.widget_positions.get("notifications"),
            Some(&WidgetPosition { x: 5.0, y: 7.0 }),
            "must not overwrite an existing entry",
        );
    }

    #[test]
    fn skips_when_neither_legacy_key_present() {
        // Fresh install: no legacy keys, no widget_positions["notifications"].
        // The helper's spec default kicks in client-side, so we should NOT
        // pollute settings.json with a redundant entry.
        let raw = json!({});
        let mut s = fresh_settings();

        let changed = apply(&raw, &mut s);

        assert!(!changed);
        assert!(s.widget_positions.get("notifications").is_none());
    }
}
