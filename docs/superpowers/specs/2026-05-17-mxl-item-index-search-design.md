# MXL Item Index Search Design

## Goal

Make the in-game MXL item search feel dynamic while keeping detailed item data on the existing targeted API request.

The current search uses `https://tsw.vn.cz/stats/api_item.php?q=<query>` for both list results and detail lookups. That endpoint is still the right source for a specific item because it returns full stats, but its list response is capped at three matches. The new `https://tsw.vn.cz/stats/api_item.php?mode=index` endpoint returns the full item index, so the search overlay can filter locally as the user types.

## API Shape

`mode=index` returns a JSON object with:

- `generated_at`: index generation timestamp.
- `count`: total item count.
- `items`: list of entries.

Each index entry contains:

- `name`
- `quality`
- `class`
- `type`

The detail endpoint remains unchanged:

- `q=<item name>` returns `items[]` with `name_display`, `runeword`, `runeword_level`, and `stats`.

## Backend Design

Extend `src-tauri/src/mxl_item_api.rs` rather than adding a second API module. This keeps all MXL item API contracts, parsing, caching, and rate-limit behavior in one place.

`MxlItemApiState` will own two independent caches:

- The existing query cache for detailed `q=` responses.
- A new in-memory index cache loaded from `mode=index` on first typeahead search.

`search_mxl_items` will choose behavior by query intent:

- For normal user typeahead, load or reuse the index and return locally filtered `MxlItemEntry` values with `detail: null`.
- For exact item detail lookups from a selected row, keep using the existing `q=<item name>` request and parse full detail as today.

To keep the command contract small, the frontend will pass a lookup mode parameter rather than adding a second Tauri command. Valid modes are `index` for typeahead and `detail` for selected item lookup. If a mode is absent, the backend defaults to `detail` so older call sites remain safe during the refactor.

## Matching

Typeahead starts when the trimmed query has at least two characters.

Local matching is case-insensitive and checks these fields:

- `name`
- `class`
- `type`
- `quality`

Ranking is intentionally simple:

- Exact name match first.
- Name prefix match next.
- Name substring match next.
- Class, type, or quality matches after name matches.
- Stable alphabetical tie-breaker by item name.

The index search does not cap results. The index has roughly 1.5k entries, so returning every local match is acceptable and avoids hiding valid items from broad searches such as `relic`.

## Frontend Design

Update `src/components/ItemSearchOverlay.svelte` to run a debounced search while the user types.

Behavior:

- Empty input clears results and message.
- One-character input shows a short hint to type at least two characters.
- Two or more characters trigger `search_mxl_items` with mode `index` after a short debounce.
- Pressing Enter still triggers an immediate index search for the current input.
- Clicking a result calls `search_mxl_items` with mode `detail` and the exact item name.
- If the overlay opens with a prefilled hovered-item name, it uses mode `detail` so a single exact item can still auto-open its tooltip.

Existing result rendering and tooltip rendering stay in place. The list rows continue to use `quality`, `class`, and `typeName`; the tooltip continues to use the full detail payload from the old endpoint.

The overlay uses local scrollbar styling in `ItemSearchOverlay.svelte` instead of the global main-window scrollbar from `src/styles/reset.css`. The result list and tooltip get a narrow dark track with a light/gold thumb so the scrollbar fits the in-game overlay visual language.

## Error Handling

Index loading failures return the existing controlled `Error` result shape with the current user-facing search failure message.

The existing request limiter remains for network calls, but local index filtering should not consume rate-limit slots after the index is loaded. Detail requests keep the existing cache and limiter behavior.

If the index payload is malformed, the backend returns a controlled error rather than exposing parser details.

## Testing

Add Rust unit tests for:

- Parsing the index response.
- Mapping index entries to `MxlItemEntry` with `detail: null`.
- Ranking exact, prefix, substring, and metadata matches.
- Returning all index matches without an artificial cap.
- Keeping detail response parsing unchanged.

Manual verification:

- Open item search overlay and type `ly`; results appear without pressing Enter.
- Type `lylia`; select `Lylia's Curse`; tooltip loads via the old detail endpoint and shows stats.
- Open search from a hovered item; exact item detail still auto-opens when available.
