use serde_json::Value;

use crate::hotkeys::HotkeyConfig;
use crate::settings::AppSettings;

pub fn apply(raw: &Value, s: &mut AppSettings) -> bool {
    let old_default = HotkeyConfig {
        key_code: 0x4E,
        modifiers: 0,
        display: "N".to_string(),
    };

    if raw.get("lootHistoryHotkey").is_some() && s.loot_history_hotkey == old_default {
        s.loot_history_hotkey = HotkeyConfig {
            key_code: 0x4E,
            modifiers: 0x0001,
            display: "Alt+N".to_string(),
        };
        return true;
    }

    false
}
