# MXL Item Index Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace capped remote list search with dynamic local typeahead backed by the new MXL item index endpoint, while keeping detailed item stats on the existing targeted `q=` endpoint.

**Architecture:** Keep the API boundary in `src-tauri/src/mxl_item_api.rs`. The Rust backend loads `mode=index` once, filters cached index entries locally for typeahead, and still calls `q=<item name>` for detail lookups. The Svelte overlay debounces typing from two characters onward and passes an explicit lookup mode to the same Tauri command.

**Tech Stack:** Rust, Tauri v2 commands, `ureq`, `serde`, Svelte 5, TypeScript, pnpm.

---

## File Structure

- Modify `src-tauri/src/mxl_item_api.rs`: add index response parsing, in-memory index cache, local matching/ranking, command mode handling, and Rust unit tests.
- Modify `src/lib/mxl-item-search.ts`: add the shared `MxlItemSearchMode` TypeScript union used by the overlay.
- Modify `src/components/ItemSearchOverlay.svelte`: add debounced typeahead from two characters, pass `mode: 'index'` for list results, and pass `mode: 'detail'` for tooltip detail requests.
- Modify `src/components/ItemSearchOverlay.svelte`: style overlay scrollbars locally so they do not inherit the main-window scrollbar look.
- No new runtime module is needed. Keeping the change in the current API module preserves the existing command registration in `src-tauri/src/main.rs`.
- Do not run `cargo fmt` or repository-wide formatter write commands. Project instructions explicitly forbid them unless the user asks for formatting in this turn.
- Do not commit during execution unless the user explicitly says `commit`, `закоммить`, or equivalent in the current turn.

## Task 1: Parse Index Responses

**Files:**
- Modify: `src-tauri/src/mxl_item_api.rs`

- [ ] **Step 1: Add failing tests for index parsing**

Add these tests inside the existing `#[cfg(test)] mod tests` block in `src-tauri/src/mxl_item_api.rs`:

```rust
    #[test]
    fn parses_index_response_entries() {
        let body = r#"{
          "generated_at": "2026-05-17T10:00:00Z",
          "count": 2,
          "items": [
            { "name": "Lylia's Curse", "quality": "Quest", "class": "Quest Charms", "type": "Lylia's Curse<br>" },
            { "name": "Azurewrath", "quality": "SU", "class": "Crystal Swords", "type": "Crystal Sword" }
          ]
        }"#;

        let entries = parse_index_response(body).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "Lylia's Curse");
        assert_eq!(entries[0].quality, "Quest");
        assert_eq!(entries[0].class, "Quest Charms");
        assert_eq!(entries[0].type_name, "Lylia's Curse<br>");
        assert_eq!(entries[0].detail, None);
    }

    #[test]
    fn invalid_index_json_returns_controlled_error() {
        let result = parse_index_response("not json");

        assert_eq!(result, Err(SEARCH_FAILED_MESSAGE.to_string()));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: FAIL because `parse_index_response` does not exist.

- [ ] **Step 3: Add index response structs and parser**

In `src-tauri/src/mxl_item_api.rs`, add these constants near the existing API constants:

```rust
const INDEX_RESULT_LIMIT: usize = 50;
const TYPEAHEAD_MIN_QUERY_LEN: usize = 2;
const TYPEAHEAD_MIN_QUERY_MESSAGE: &str = "Type at least 2 characters to search.";
```

Add these structs after `RawMatch`:

```rust
#[derive(Debug, Deserialize)]
struct RawIndexResponse {
    count: usize,
    items: Vec<RawIndexItem>,
}

#[derive(Debug, Deserialize)]
struct RawIndexItem {
    name: String,
    quality: String,
    class: String,
    #[serde(rename = "type", default)]
    type_name: String,
}
```

Add this parser near `parse_api_response`:

```rust
fn parse_index_response(body: &str) -> Result<Vec<MxlItemEntry>, String> {
    let parsed: RawIndexResponse = serde_json::from_str(body)
        .map_err(|_| SEARCH_FAILED_MESSAGE.to_string())?;

    let entries = parsed
        .items
        .into_iter()
        .map(|item| MxlItemEntry {
            name: item.name,
            quality: item.quality,
            class: item.class,
            type_name: item.type_name,
            detail: None,
        })
        .collect::<Vec<_>>();

    if parsed.count != entries.len() {
        return Err(SEARCH_FAILED_MESSAGE.to_string());
    }

    Ok(entries)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: PASS for both tests.

- [ ] **Step 5: Review diff checkpoint**

Run:

```bash
git diff -- src-tauri/src/mxl_item_api.rs
```

Expected: diff only adds index parsing support and tests. Do not commit unless the user explicitly asks for a commit in the current turn.

## Task 2: Add Local Index Search

**Files:**
- Modify: `src-tauri/src/mxl_item_api.rs`

- [ ] **Step 1: Add failing tests for local search ranking and caps**

Add these tests inside the existing `#[cfg(test)] mod tests` block:

```rust
    fn index_entry(name: &str, quality: &str, class: &str, type_name: &str) -> MxlItemEntry {
        MxlItemEntry {
            name: name.to_string(),
            quality: quality.to_string(),
            class: class.to_string(),
            type_name: type_name.to_string(),
            detail: None,
        }
    }

    #[test]
    fn searches_index_with_name_first_ranking() {
        let entries = vec![
            index_entry("Blade of Light", "SU", "Swords", "Sword"),
            index_entry("Lylia's Curse", "Quest", "Quest Charms", "Charm"),
            index_entry("Curse of the Zakarum", "SU", "Maces", "Mace"),
            index_entry("Arcane Hunger", "Effigy", "Occult Effigies", "Occult Effigy"),
        ];

        let result = search_index_entries("curse", &entries);

        match result {
            MxlItemSearchResult::Results { entries, message, .. } => {
                assert_eq!(message, None);
                assert_eq!(entries[0].name, "Curse of the Zakarum");
                assert_eq!(entries[1].name, "Lylia's Curse");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn searches_index_metadata_after_name_matches() {
        let entries = vec![
            index_entry("Questing Beast", "SU", "Swords", "Sword"),
            index_entry("Lylia's Curse", "Quest", "Quest Charms", "Charm"),
            index_entry("Sunstone", "Quest", "Charms", "Charm"),
        ];

        let result = search_index_entries("quest", &entries);

        match result {
            MxlItemSearchResult::Results { entries, .. } => {
                assert_eq!(entries[0].name, "Questing Beast");
                assert_eq!(entries[1].name, "Lylia's Curse");
                assert_eq!(entries[2].name, "Sunstone");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn short_index_query_returns_hint() {
        let entries = vec![index_entry("Lylia's Curse", "Quest", "Quest Charms", "Charm")];

        let result = search_index_entries("l", &entries);

        assert_eq!(
            result,
            MxlItemSearchResult::Results {
                query: "l".to_string(),
                entries: Vec::new(),
                message: Some(TYPEAHEAD_MIN_QUERY_MESSAGE.to_string()),
            }
        );
    }

    #[test]
    fn index_search_caps_results_and_reports_overflow() {
        let entries = (0..(INDEX_RESULT_LIMIT + 2))
            .map(|i| index_entry(&format!("Sacred Item {:03}", i), "SU", "Swords", "Sword"))
            .collect::<Vec<_>>();

        let result = search_index_entries("sacred", &entries);

        match result {
            MxlItemSearchResult::Results { entries, message, .. } => {
                assert_eq!(entries.len(), INDEX_RESULT_LIMIT);
                assert_eq!(message, Some(TOO_MANY_MATCHES_MESSAGE.to_string()));
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: FAIL because `search_index_entries` does not exist.

- [ ] **Step 3: Add local search helper functions**

Add these helper functions near `normalize_query` and `percent_encode_query`:

```rust
fn normalized_contains(value: &str, query: &str) -> bool {
    value.to_lowercase().contains(query)
}

fn index_match_rank(entry: &MxlItemEntry, query: &str) -> Option<u8> {
    let name = entry.name.to_lowercase();
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    if name.contains(query) {
        return Some(2);
    }
    if normalized_contains(&entry.class, query)
        || normalized_contains(&entry.type_name, query)
        || normalized_contains(&entry.quality, query)
    {
        return Some(3);
    }
    None
}

fn search_index_entries(query: &str, entries: &[MxlItemEntry]) -> MxlItemSearchResult {
    let trimmed = query.trim();
    let normalized = normalize_query(trimmed);

    if normalized.len() < TYPEAHEAD_MIN_QUERY_LEN {
        return MxlItemSearchResult::Results {
            query: trimmed.to_string(),
            entries: Vec::new(),
            message: Some(TYPEAHEAD_MIN_QUERY_MESSAGE.to_string()),
        };
    }

    let mut matches = entries
        .iter()
        .filter_map(|entry| index_match_rank(entry, &normalized).map(|rank| (rank, entry)))
        .collect::<Vec<_>>();

    matches.sort_by(|(left_rank, left), (right_rank, right)| {
        left_rank
            .cmp(right_rank)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    let overflow = matches.len() > INDEX_RESULT_LIMIT;
    let entries = matches
        .into_iter()
        .take(INDEX_RESULT_LIMIT)
        .map(|(_, entry)| entry.clone())
        .collect::<Vec<_>>();

    MxlItemSearchResult::Results {
        query: trimmed.to_string(),
        entries,
        message: overflow.then(|| TOO_MANY_MATCHES_MESSAGE.to_string()),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: PASS for all four tests.

- [ ] **Step 5: Review diff checkpoint**

Run:

```bash
git diff -- src-tauri/src/mxl_item_api.rs
```

Expected: diff includes local search helpers and tests. Do not commit unless the user explicitly asks for a commit in the current turn.

## Task 3: Wire Backend Command Modes and Index Cache

**Files:**
- Modify: `src-tauri/src/mxl_item_api.rs`

- [ ] **Step 1: Add failing tests for mode parsing**

Add these tests inside the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn search_mode_defaults_to_detail() {
        assert_eq!(MxlItemSearchMode::from_optional(None), MxlItemSearchMode::Detail);
    }

    #[test]
    fn search_mode_accepts_index_case_insensitively() {
        assert_eq!(
            MxlItemSearchMode::from_optional(Some("INDEX".to_string())),
            MxlItemSearchMode::Index
        );
    }

    #[test]
    fn search_mode_unknown_value_uses_detail() {
        assert_eq!(
            MxlItemSearchMode::from_optional(Some("unknown".to_string())),
            MxlItemSearchMode::Detail
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: FAIL because `MxlItemSearchMode` does not exist.

- [ ] **Step 3: Add mode enum, cache field, and index fetch path**

Add this enum after `MxlItemSearchResult`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MxlItemSearchMode {
    Detail,
    Index,
}

impl MxlItemSearchMode {
    fn from_optional(value: Option<String>) -> Self {
        match value.as_deref().map(str::trim) {
            Some(mode) if mode.eq_ignore_ascii_case("index") => Self::Index,
            _ => Self::Detail,
        }
    }
}
```

Update `MxlItemApiState`:

```rust
pub struct MxlItemApiState {
    cache: Mutex<HashMap<String, MxlItemSearchResult>>,
    index_cache: Mutex<Option<Vec<MxlItemEntry>>>,
    limiter: Mutex<RequestWindow>,
    agent: ureq::Agent,
}
```

Update `Default for MxlItemApiState`:

```rust
impl Default for MxlItemApiState {
    fn default() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            index_cache: Mutex::new(None),
            limiter: Mutex::new(RequestWindow::default()),
            agent: default_agent(),
        }
    }
}
```

Add this network helper after `fetch_from_api`:

```rust
fn fetch_index_from_api(agent: &ureq::Agent) -> Result<Vec<MxlItemEntry>, String> {
    let url = format!("{}?mode=index", API_URL);
    let body = match agent.get(&url).call() {
        Ok(response) => response
            .into_body()
            .read_to_string()
            .map_err(|_| SEARCH_FAILED_MESSAGE.to_string())?,
        Err(_) => return Err(SEARCH_FAILED_MESSAGE.to_string()),
    };

    parse_index_response(&body)
}
```

Add this method inside `impl MxlItemApiState`:

```rust
    fn search_index(&self, query: &str, now: Instant) -> MxlItemSearchResult {
        match self.index_cache.lock() {
            Ok(cache) => {
                if let Some(entries) = cache.as_ref() {
                    return search_index_entries(query, entries);
                }
            }
            Err(_) => {
                return MxlItemSearchResult::Error {
                    query: query.trim().to_string(),
                    message: SEARCH_FAILED_MESSAGE.to_string(),
                }
            }
        }

        match self.limiter.lock() {
            Ok(mut limiter) => {
                if let Err(retry_after_ms) = limiter.check(now) {
                    return MxlItemSearchResult::RateLimited {
                        message: RATE_LIMIT_MESSAGE.to_string(),
                        retry_after_ms,
                    };
                }
            }
            Err(_) => {
                return MxlItemSearchResult::Error {
                    query: query.trim().to_string(),
                    message: SEARCH_FAILED_MESSAGE.to_string(),
                }
            }
        }

        let entries = match fetch_index_from_api(&self.agent) {
            Ok(entries) => entries,
            Err(message) => {
                return MxlItemSearchResult::Error {
                    query: query.trim().to_string(),
                    message,
                }
            }
        };

        let result = search_index_entries(query, &entries);
        if let Ok(mut cache) = self.index_cache.lock() {
            *cache = Some(entries);
        }
        result
    }
```

Update the Tauri command signature and implementation:

```rust
#[tauri::command]
pub fn search_mxl_items(
    query: String,
    mode: Option<String>,
    state: tauri::State<MxlItemApiState>,
) -> MxlItemSearchResult {
    let now = Instant::now();
    match MxlItemSearchMode::from_optional(mode) {
        MxlItemSearchMode::Detail => state.cached_or_fetch(&query, now),
        MxlItemSearchMode::Index => state.search_index(&query, now),
    }
}
```

- [ ] **Step 4: Run all Rust item API tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: PASS for all `mxl_item_api` tests.

- [ ] **Step 5: Review diff checkpoint**

Run:

```bash
git diff -- src-tauri/src/mxl_item_api.rs
```

Expected: diff includes command mode support and index cache. Do not commit unless the user explicitly asks for a commit in the current turn.

## Task 4: Add Frontend Search Modes

**Files:**
- Modify: `src/lib/mxl-item-search.ts`
- Modify: `src/components/ItemSearchOverlay.svelte`

- [ ] **Step 1: Add TypeScript mode type**

In `src/lib/mxl-item-search.ts`, add this after `MxlItemSearchResult`:

```ts
export type MxlItemSearchMode = 'index' | 'detail';
```

- [ ] **Step 2: Import the mode type in the overlay**

Update the import block in `src/components/ItemSearchOverlay.svelte`:

```ts
  import {
    itemQualityColor,
    type MxlItemDetail,
    type MxlItemEntry,
    type MxlItemSearchMode,
    type MxlItemSearchResult,
    type OpenItemSearchPayload,
  } from '../lib/mxl-item-search';
```

- [ ] **Step 3: Add debounce state and constants**

Add these near the existing local state declarations in `ItemSearchOverlay.svelte`:

```ts
  const TYPEAHEAD_DELAY_MS = 180;
  const MIN_TYPEAHEAD_CHARS = 2;

  let typeaheadTimer: number | null = null;
  let skipTypeaheadValue: string | null = null;
```

- [ ] **Step 4: Update search function to pass backend mode**

Replace `runSearch` with this implementation:

```ts
  async function runSearch(
    query = inputValue,
    mode: MxlItemSearchMode = 'index',
    shouldAutoOpenSingleDetailedResult = false,
  ) {
    const q = query.trim();
    if (!q) return;
    const requestId = ++searchRequestId;
    detailRequestId += 1;
    loading = true;
    loadingName = null;
    tooltip = null;
    message = '';
    try {
      const result = await invoke<MxlItemSearchResult>('search_mxl_items', { query: q, mode });
      if (requestId !== searchRequestId) return;
      applyResult(result, shouldAutoOpenSingleDetailedResult);
    } catch (err) {
      if (requestId !== searchRequestId) return;
      console.error('[ItemSearch] search failed:', err);
      entries = [];
      message = 'Search failed. Check connection and try again.';
    } finally {
      if (requestId === searchRequestId) loading = false;
    }
  }
```

- [ ] **Step 5: Update prefilled open behavior to use detail mode**

Replace the body of `openSearch` with:

```ts
  async function openSearch(query: string | null) {
    open = true;
    tooltip = null;
    inputValue = query ?? '';
    message = '';
    autoOpenSingleDetailedResult = Boolean(query && query.trim());
    skipTypeaheadValue = query?.trim() || null;
    await onActiveChange(true);
    focusInput();
    if (query && query.trim()) {
      void runSearch(query, 'detail', autoOpenSingleDetailedResult);
      autoOpenSingleDetailedResult = false;
    }
  }
```

- [ ] **Step 6: Update form submit to force immediate index search**

Replace the form `onsubmit` handler with:

```svelte
          onsubmit={(event) => {
            event.preventDefault();
            if (typeaheadTimer !== null) {
              window.clearTimeout(typeaheadTimer);
              typeaheadTimer = null;
            }
            skipTypeaheadValue = null;
            void runSearch(inputValue, 'index');
          }}
```

- [ ] **Step 7: Update tooltip detail lookup to use detail mode**

Replace the invoke line inside `openTooltip` with:

```ts
      const result = await invoke<MxlItemSearchResult>('search_mxl_items', {
        query: entry.name,
        mode: 'detail',
      });
```

- [ ] **Step 8: Run Svelte typecheck**

Run:

```bash
pnpm check
```

Expected: PASS with no TypeScript or Svelte diagnostics caused by these changes.

- [ ] **Step 9: Review diff checkpoint**

Run:

```bash
git diff -- src/lib/mxl-item-search.ts src/components/ItemSearchOverlay.svelte
```

Expected: diff only adds mode typing and mode-aware searches. Do not commit unless the user explicitly asks for a commit in the current turn.

## Task 5: Add Debounced Typeahead UI Behavior

**Files:**
- Modify: `src/components/ItemSearchOverlay.svelte`

- [ ] **Step 1: Add the input-driven typeahead effect**

Add this `$effect` after the existing `$effect(() => { onActiveChange(active); });` block:

```ts
  $effect(() => {
    const value = inputValue;
    if (!open) return;

    if (typeaheadTimer !== null) {
      window.clearTimeout(typeaheadTimer);
      typeaheadTimer = null;
    }

    const q = value.trim();
    if (!q) {
      searchRequestId += 1;
      detailRequestId += 1;
      entries = [];
      tooltip = null;
      loading = false;
      loadingName = null;
      message = '';
      skipTypeaheadValue = null;
      return;
    }

    if (skipTypeaheadValue && q === skipTypeaheadValue) {
      return;
    }
    skipTypeaheadValue = null;

    if (q.length < MIN_TYPEAHEAD_CHARS) {
      searchRequestId += 1;
      detailRequestId += 1;
      entries = [];
      tooltip = null;
      loading = false;
      loadingName = null;
      message = 'Type at least 2 characters to search.';
      return;
    }

    typeaheadTimer = window.setTimeout(() => {
      typeaheadTimer = null;
      void runSearch(q, 'index');
    }, TYPEAHEAD_DELAY_MS);

    return () => {
      if (typeaheadTimer !== null) {
        window.clearTimeout(typeaheadTimer);
        typeaheadTimer = null;
      }
    };
  });
```

- [ ] **Step 2: Clear timer on close**

Add this near the top of `closeAll` after `detailRequestId += 1;`:

```ts
    if (typeaheadTimer !== null) {
      window.clearTimeout(typeaheadTimer);
      typeaheadTimer = null;
    }
```

Add this near the end of `closeAll` after `message = '';`:

```ts
    skipTypeaheadValue = null;
```

- [ ] **Step 3: Update placeholder text**

Replace the input placeholder:

```svelte
            placeholder="Type at least 2 characters"
```

- [ ] **Step 4: Run Svelte typecheck**

Run:

```bash
pnpm check
```

Expected: PASS with no Svelte diagnostics.

- [ ] **Step 5: Review diff checkpoint**

Run:

```bash
git diff -- src/components/ItemSearchOverlay.svelte
```

Expected: diff adds debounced typeahead and cleanup. Do not commit unless the user explicitly asks for a commit in the current turn.

## Task 6: Final Verification

**Files:**
- Verify: `src-tauri/src/mxl_item_api.rs`
- Verify: `src/lib/mxl-item-search.ts`
- Verify: `src/components/ItemSearchOverlay.svelte`

- [ ] **Step 1: Run all item API tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api
```

Expected: PASS for all item API tests.

- [ ] **Step 2: Run frontend typecheck**

Run:

```bash
pnpm check
```

Expected: PASS.

- [ ] **Step 3: Build frontend bundle**

Run:

```bash
pnpm build
```

Expected: PASS. This runs `svelte-check --tsconfig ./tsconfig.json && vite build` and does not invoke `cargo fmt`.

- [ ] **Step 4: Manual behavior check in development app**

Run:

```bash
pnpm tauri dev
```

Expected manual results:

- Open the item search overlay.
- Type `l`; the overlay shows `Type at least 2 characters to search.`
- Type `ly`; results appear without pressing Enter.
- Type `lylia`; `Lylia's Curse` appears in the list.
- Click `Lylia's Curse`; tooltip stats load from the old `q=` detail endpoint.
- Open item search from a hovered in-game item; exact detail lookup still auto-opens the tooltip when the old API returns one detailed item.

- [ ] **Step 5: Inspect final diff**

Run:

```bash
git diff -- src-tauri/src/mxl_item_api.rs src/lib/mxl-item-search.ts src/components/ItemSearchOverlay.svelte docs/superpowers/specs/2026-05-17-mxl-item-index-search-design.md docs/superpowers/plans/2026-05-17-mxl-item-index-search.md
```

Expected: diff contains only the item index search feature and the planning documents. Do not commit unless the user explicitly asks for a commit in the current turn.

## Follow-up Task 7: Remove Index Result Cap And Restyle Overlay Scrollbars

**Files:**
- Modify: `src-tauri/src/mxl_item_api.rs`
- Modify: `src/components/ItemSearchOverlay.svelte`

- [ ] **Step 1: Write the failing Rust test**

Replace the cap test with a test that creates `INDEX_RESULT_LIMIT + 2` matching entries and expects all matches to be returned with no overflow message.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api`

Expected: FAIL because current code still takes only `INDEX_RESULT_LIMIT` entries and emits `TOO_MANY_MATCHES_MESSAGE`.

- [ ] **Step 3: Remove cap implementation**

Remove `INDEX_RESULT_LIMIT`, `TOO_MANY_MATCHES_MESSAGE`, overflow calculation, and `.take(INDEX_RESULT_LIMIT)` from index search.

- [ ] **Step 4: Add overlay-local scrollbar CSS**

Add classes/selectors in `ItemSearchOverlay.svelte` so `.results` and `.item-tooltip` use a narrow overlay scrollbar independent from global scrollbar styles.

- [ ] **Step 5: Verify**

Run `cargo test --manifest-path src-tauri/Cargo.toml mxl_item_api`, `pnpm check`, and `pnpm build`.

## Self-Review

- Spec coverage: backend index loading, local filtering, detail endpoint preservation, typeahead from two characters, error handling, and verification are each covered by tasks above.
- Placeholder scan: the plan uses concrete file paths, command lines, constants, and code snippets. It does not use open-ended implementation placeholders.
- Type consistency: Rust uses `MxlItemEntry`, `MxlItemSearchResult`, and optional command `mode`; TypeScript uses `MxlItemSearchMode = 'index' | 'detail'`; Svelte sends the same mode strings to the Tauri command.
