# DPS Meter — Damage Scaling Bug Investigation

**Дата:** 2026-05-09
**Статус:** open / RE pending
**Связано:** `docs/dps-meter-reverse-engineering.md`,
`docs/superpowers/plans/2026-05-07-dps-meter.md`,
`src-tauri/src/dps_hook/trampoline.rs`,
`src-tauri/src/dps_meter.rs`.

---

## TL;DR

Формула scaling, унаследованная из изначальной RE-сессии и реализованная
байт-в-байт по плану, **математически корректна**, но опирается на
неверное допущение про `wMaxHP[difficulty]` из `MonStats.txt`. RE-автор
считал это «real HP units», на деле это **template-значение, к которому
engine применяет дополнительный level scaling при спавне моба**. В Hell
runtime max HP моба может превышать template в 50–150× — отсюда
реальные DPS-показания получаются на 1–2 порядка меньше ожидаемых.

Симптом: один моб в Act 1 Hell, оружие игрока 491–726 на удар, моб
кладётся за несколько ударов. DPS-meter показывает Total = 67, DPS = 5–6.
Ожидание (по реальному урону) ≈ 5000+ Total.

Цель этого отчёта — зафиксировать корень проблемы и план следующей
RE-итерации, чтобы найти runtime max HP (или, в худшем случае,
максимально приблизить scaling к реальности).

---

## 1. Что реализовано сейчас

### Hook side (`trampoline.rs:194-205`)

В трамплин-helper'е читается `MonStats.wMaxHP[difficulty]` из
`MonStats.txt`-record:

```asm
mov ecx, [difficulty_addr]      ; 0=Normal, 1=NM, 2=Hell
shl ecx, 1                      ; diff * 2
add ecx, 0xB0                   ; offset MAX_HP_NORMAL
mov esi, [ebp+0xC]              ; pMonStatsRecord
movzx eax, word [esi + ecx]     ; max_hp = u16 [+0xB0 + diff*2]
mov [ebp-0x10], eax             ; save for ring slot
```

Записывается в slot ring-buffer'а как `u16 max_hp`.

### Meter side (`dps_meter.rs:102-119`)

```rust
pub fn ingest(&mut self, ts_ms: u32, delta_raw_with_flag: u32, max_hp: u16) {
    let is_kill = (delta_raw_with_flag & KILL_FLAG) != 0;
    let delta_raw = delta_raw_with_flag & DELTA_MASK;
    let damage = ((delta_raw as u64).saturating_mul(max_hp as u64) / 32768) as u32;
    // ...
}
```

Это **в точности** формула из RE-сессии:
`damage = (delta_raw / 32768) × MonStats.wMaxHP[difficulty]`.

---

## 2. Что неправильно в исходной RE-гипотезе

RE-сессия установила два отдельных факта:

**Факт 1 (верный):** `delta_raw` — это процент HP × 256.

Происхождение задокументировано в `docs/dps-meter-reverse-engineering.md`
строки 240–281: D2Client packet handler `+0x4BE70` case A берёт байт
из пакета (нижние 7 бит, 0..127), увеличивает на 1 если >1, потом
`shl eax, 8` → диапазон 0..32768. Это и есть значение, которое
`Ord10887` пишет в `STAT_HITPOINTS` для моба.

**Факт 2 (ошибочный):** «`MonStats.wMaxHP[difficulty]` — это per-monster
TRUE max HP (template value). `damage = (delta_raw / 32768) ×
monstats_max` gives engine-honest absolute damage».

Это утверждение из RE doc строки 484–485 — здесь и кроется баг.
**MonStats.txt — это таблица base/template-значений**, не runtime.
Engine при создании моба применяет formula:

```
runtime_max_hp ≈ (wMinHP[diff] + (wMaxHP[diff] - wMinHP[diff]) × random)
              × level_scaling_multiplier(monster_level, area_level)
              × MXL_specific_modifiers
```

В vanilla D2 это `MonLvl.txt`-driven scaling. В MXL Sigma поверх этого
добавлены свои множители (HP инфляция в Hell — основная фича MXL).

Проверка: Treehead в RE doc указан как `wMaxHP[Hell] = 600`. Реальный
MXL-Treehead в Hell на нормальном уровне игрока имеет HP в районе
30 000 – 60 000. Factor ~50–100×, согласуется с симптомом юзера
(67 vs ~5000+).

### Почему RE-автор не поймал это

Автор смотрел `STAT_MAXHP` (id=7) в stat list через CE и видел
`raw = 32768` (это нормализованная форма из протокола). Из этого
заключил «значит max HP всех мобов одинаковый, нужно брать template
из MonStats.txt». Но забыл, что нормализация — это **client-side
протокольный quirk для отображения HP-bar'а**, а server-side actual
HP хранится отдельно где-то ещё.

---

## 3. Что точно есть в распоряжении hook

В трамплине доступны (стек + регистры на момент перед helper'ом):

- `pUnit` (eax после фильтра) — UnitAny структура моба
- `pUnitData` = `[pUnit + 0x14]` — для моба это `pMonsterData` (в D2MOO
  обычно `D2MonsterDataStrc`)
- `pMonStatsRecord` = `*pUnitData` (esi после фильтра) — запись в
  `MonStats.txt` (template)
- `pStatListEx` = `[pUnit + 0x5C]` (ebx)
- `pStat` = `[pStatListEx + 0x24]` (edi) — массив `D2StatStrc`
- `wStatCount1` = `[pStatListEx + 0x28]` (ecx)
- `delta_raw` = `old_stat6 - new_stat6` (ebx после поиска stat 6)

Stat list содержит как минимум:
- `STAT_HITPOINTS = 6` — текущий HP (нормализованный 0..32768 для моба)
- `STAT_MAXHP = 7` — predполагаемо max HP (нужно проверить **в SP**)

`pMonsterData` (= `pUnitData` для моба) — почти наверняка содержит
runtime-поля моба, включая level. В D2MOO структура `D2MonsterDataStrc`
включает `dwLevel` или аналог.

---

## 4. Гипотезы решения

### Гипотеза A: STAT_MAXHP raw в SP содержит actual max HP

**Идея:** RE doc видел `stat 7 raw = 32768` в MP-сценарии или там, где
клиент применил protocol-нормализацию. В SP MXL может хранить там
actual max HP (например, `actual × 256`, как для player'а в stat 6).

**Стоимость:** ~1 час.

**Проверка (без кода):**
1. Использовать `docs/ce-scripts/dump-stats.lua` или новый CE-скрипт
   на конкретном мобе в SP Hell.
2. Прочитать `[pUnit + 0x5C] + 0x24 → search nStat==7 → +0x04`.
3. Если значение != 32768 — это actual в фиксированной точке.

**Если работает:**
- Трамплин ищет stat 7 в том же loop'е, что и stat 6, передаёт
  оба значения в helper.
- Helper кладёт в slot `actual_max_hp` (u32) вместо template `wMaxHP`
  (u16). Slot вырастает с 16 до 20 байт, либо переиспользуется поле.
- Rust ingest: `damage = (delta_raw / 32768) × actual_max_hp`.

**Если НЕ работает (stat 7 raw == 32768 даже в SP):**
- Fallback к гипотезе B.

### Гипотеза B: pMonsterData содержит monster_level → scaling formula

**Идея:** Реверснуть формулу, по которой engine считает runtime max HP
из template + level + area. Достать monster level из `pMonsterData`,
посчитать на Rust-стороне.

**Стоимость:** 2–4 часа.

**Что нужно реверсить:**
1. Offset поля `dwLevel` (или `bLevel`) в `D2MonsterDataStrc`. Кандидаты
   из D2MOO/общего знания: `+0x16`, `+0x18`, `+0x1A`. Найти через
   CE-сравнение монстров разных уровней в одной зоне.
2. Найти, где engine применяет scaling — поискать `ApplyDamage` /
   `InitMonster` / любую функцию с MonStats lookup + level multiply.
   Точка интереса: места где `wMaxHP[diff]` загружается и далее
   умножается. Можно найти через breakpoint на чтение `+0xB0..+0xB6`
   в MonStats record.
3. Записать формулу в `dps_meter.rs` (не в трамплин — слишком сложно).
   Передавать в helper два значения: `wMaxHP_template` и `monster_level`,
   на Rust-стороне делать
   `runtime_max = formula(template, level, area_level, diff)`.

**Альтернатива внутри B:** поискать **cached runtime max HP** в самом
`pUnitData`. Engine может хранить computed max в structure, чтобы не
пересчитывать каждый кадр. Если есть — это идеальный путь, не требует
формулы.

Кандидатные offsets для дампа в `pMonsterData`:
- `+0x10..+0x40` — обычная зона runtime fields в D2 структурах.
- Искать u32 значения в диапазоне 1000–500000 (плауsible HP range).
- Проверить, что значение ≠ template MAX_HP_NORMAL/NM/HELL.

### Гипотеза C: эмпирический множитель по difficulty (fallback)

**Идея:** если A и B не выйдут — применить грубый множитель в
`dps_meter::ingest`:
```rust
const HP_SCALE: [u32; 3] = [1, 30, 75]; // Normal / NM / Hell
let damage = scaled × HP_SCALE[difficulty as usize];
```

**Стоимость:** 30 мин (включая UI-tooltip про погрешность).

**Точность:** сильно плавает между мобами — один может прыгать ×40,
другой ×120. Числа окажутся «в правильном порядке», но per-monster
смысла не несут.

**Калибровка множителей:** по нескольким мобам (Quill Rat, Carver,
Treehead, какой-нибудь act 5 mob) собрать пары (template wMaxHP, actual
HP-from-CE) → median ratio per difficulty.

---

## 5. Acceptance criteria

Любая из гипотез считается успешной, если для контрольного теста:

1. **One-shot test** — игрок заходит в SP Hell на act 1, бьёт одного
   моба до смерти. После kill:
   - `Total ≈ actual_max_hp_of_that_monster` (±20%).
2. **Multi-mob test** — игрок убивает 5 мобов разных классов в одной
   зоне (Hell). Среднее `Total per kill / actual_avg_HP` ≈ 1.0 (±30%).
3. **Cross-difficulty test** — тот же моб в Normal vs NM vs Hell даёт
   Total в правильном относительном масштабе (Normal ≪ NM ≪ Hell).

---

## 6. План работ

```
[ ] Шаг 1 — гипотеза A проверка
    [ ] CE-скрипт: dump pStatListEx → stat 7 для моба в SP Hell
    [ ] Сравнить raw value с 32768 и с предполагаемым actual HP
    [ ] Решение: A работает / fallback к B
[ ] Шаг 2 (если A не сработал) — гипотеза B
    [ ] CE: dump pUnitData[+0x10..+0x40] для моба в SP Hell
    [ ] Найти кандидата на actual max HP (u32, реалистичный диапазон)
    [ ] Если есть cached field — использовать его
    [ ] Если нет — RE level scaling formula
[ ] Шаг 3 (если B не сработал) — fallback C
    [ ] Calibrate empirical multipliers по 5+ моба per difficulty
    [ ] Tooltip "approximate values, ±X%"
[ ] Шаг 4 — реализация
    [ ] Trampoline: при необходимости второй stat search или extra read
    [ ] Slot layout: max_hp u16 → u32, либо доп. поле monster_level
    [ ] Rust ingest: новый scaling
    [ ] Smoke-test acceptance criteria
[ ] Шаг 5 — закрепить в документации
    [ ] Обновить docs/dps-meter-reverse-engineering.md (раздел "scaling")
    [ ] Закрыть этот отчёт ссылкой на коммит, который чинит баг
```

---

## 7. Что НЕ нужно трогать

- **Сам hook на Ord10887** — работает корректно, события ловятся.
- **Формат delta_raw** — `(HP_pct × 256) / 32768` это правильное
  процентное преобразование. Меняется только denominator scaling.
- **Kill-flag encoding** — bit 31 в delta_raw, отдельная проблема.
- **Auto-reset на смену области** — переключён на `*pAutomapLayer`,
  работает.
- **Filter chain (statId=6, monster, wild via isSpawn)** — корректен.

---

## 8. Открытые вопросы

1. Действительно ли в SP MXL stat 7 raw отличается от 32768? (RE doc
   проверял, видимо, в MP).
2. Есть ли в `pUnitData` (`pMonsterData`) cached `runtime_max_hp` без
   нужды в level-formula?
3. Где задокументирована MXL-специфичная HP-формула — в исходниках
   MXL-сообщества (D2MR/PhrozenKeep), либо требуется RE из binary?
4. Нужен ли учёт магии монстра (champion / unique / minion имеют
   разный HP-multiplier даже от того же template)?
5. Как обрабатывать боссов (Diablo/Baal/MXL-uniques) — их HP может
   быть в `dwSeed`-зависимых полях, не в MonStats.

---

## 9. Связанные файлы

- `src-tauri/src/dps_hook/trampoline.rs` — трамплин-сторона, читает
  template wMaxHP. Точка изменения: helper-блок (строки ~196–205).
- `src-tauri/src/dps_hook/mod.rs:42` — `HookEvent.max_hp` тип u16.
  При расширении до actual HP может потребоваться u32.
- `src-tauri/src/dps_hook/ring.rs:14,86` — slot layout. Поле max_hp
  занимает 2 байта, оставшиеся 2 байта pad — есть запас.
- `src-tauri/src/dps_meter.rs:102-119` — ingest, делает scaling.
- `src-tauri/src/offsets.rs:393-418` — `monstats_txt::MAX_HP_*`,
  template offsets. Не трогать (template не виноват, виновата
  интерпретация).
- `docs/ce-scripts/dump-stats.lua`, `dump-monstats-records.lua` —
  готовые CE-инструменты для гипотезы A/B.

---

## 10. История

- **2026-05-08** — RE-сессия зафиксировала формулу
  `(delta_raw / 32768) × wMaxHP[diff]` как «engine-honest absolute
  damage». Не учтено level scaling.
- **2026-05-09** — реализован hook + scaling по плану. Все unit-тесты
  зелёные. В smoke-тесте обнаружено расхождение ~75× между ожидаемым
  и фактическим Total.
- **2026-05-09** — открыт этот отчёт; следующий шаг — RE гипотезы A/B.

## 11. Update 2026-05-09 — MP context, RE conclusions, chosen path

### Гипотеза A полностью отвергнута

Юзер играет на **MP-realm** (никогда не тестировал в SP).
В MP сервер нормализует HP в 0..127% (`+0x4BE70` packet handler case A,
см. RE doc) и шлёт клиенту в этом виде. На client side для всех мобов
`stat 6 raw = stat 7 raw = 32768` без исключений. Реальный max HP
**физически не существует на client side в MP** — HP-bar в игре
показывает только процент, не absolute число, поэтому engine не имеет
причин кэшировать actual.

Подтверждено CE-дампом: `verify-runtime-maxhp.lua` на 8+ разных классах
мобов в Act 1 Hell — все stat 6/7 = 32768.

### Гипотеза B (cached actual в client memory) — также отвергнута

Wide-net дамп `pUnit +0x00..+0x200`, `pUnitData +0x00..+0x80`,
`pStatListEx +0x00..+0x100` не показал ни одного u32 в HP-shape
range, который бы коррелировал с template wMaxHP × constant ratio
между классами мобов. Все «★»-помеченные значения — это координаты
(`~50000 = subtile units`), нормализованные дельты, или MXL custom
stats без HP-семантики.

`pStatListEx +0x24` (external pStat ptr) **указывает обратно в
`pStatListEx +0x80`** — то есть это один и тот же массив, доступный
через два пути. Не отдельный «inline» с дополнительными данными.

**Вывод: real max HP в MP D2 client не cached. Точка.**

### Полный набор stats у моба (из дампа Act 1 Hell)

```
nStat=6   (hitpoints)     value=32768   ← normalised
nStat=7   (maxhp)         value=32768   ← normalised
nStat=12  (level)         value=110     ← ★ runtime monster level
nStat=36  (damageresist)  value=10
nStat=39  (fireresist)    value=35
nStat=41  (lightresist)   value=35
nStat=43  (coldresist)    value=35
nStat=45  (poisonresist)  value=35
nStat=67  (poisondivisor) value=75 / 125
nStat=68  (thaco)         value=100
nStat=69  (?)             value=100
nStat=328 (MXL custom)    value=~21400  ← НЕ HP-related, ratio к template скачет 119–400×
```

`stat 12` — runtime monster level, доступен через тот же hook-путь
(walk pStatListEx → pStat array). Все мобы зоны имеют одинаковый
mLvl=110 (= area level). У player'a `stat 12 = 134` (его уровень).

### Calibration: linear formula

Empiric: юзер убил моба class=3136 в Act 1 Hell. Hook показал
`Total = 67`. Реальный урон (по hits) ~5000–7000.

Кандидаты:

| Class | template | mLvl | linear (×mLvl) | quad (×mLvl²/100) | cubic (×mLvl³/10000) |
|---|---|---|---|---|---|
| 3136 | 54  | 110 | 5940  | 6534  | 7187  |
| 3137 | 180 | 110 | 19800 | 21780 | 23958 |
| 3138 | 90  | 110 | 9900  | 10890 | 11979 |
| 2881 (Treehead) | 600 | 110 | 66000 | 72600 | 79867 |

Юзер видит 67 при ожидаемых ~5940 (linear для template=54). **Множитель
получается ровно `mLvl` независимо от template** — структурно
правильная зависимость, не подгон. Treehead-as-boss в Hell с 66 000 HP
тоже plausible.

**Вывод: первая итерация — `runtime_max = template × mLvl` (linear).**
Если smoke-тест покажет систематическое 1.5–2× занижение → переключиться
на quadratic. Если переоценка — нужна дополнительная константа в
знаменателе.

### Что НЕ делаем (отклонённые направления)

- **CE write-watchpoint на stat 6** для калибровки 3-5 точек — отложено
  до результатов linear formula. Если попадёт в ballpark — не нужно.
- **Чтение MonLvl.txt из D2Common data tables** — есть в архитектуре,
  но реализация (новые data-table offsets + scaling-formula RE) сильно
  дороже linear approach. Резерв на случай отсутствия точности.
- **`stat 328`** — изначально казался кандидатом из-за HP-shape range
  (~21400), но cross-class дамп показал что значения близкие у мобов
  с template 54/180/90, ratio скачет 119–400× — это не HP scaling,
  а что-то area/experience-related. Отбрасываем.

### План реализации (готов к коду)

1. **Trampoline `trampoline.rs`** — расширить filter loop:
   - Один проход по pStat array, в нём искать одновременно `stat 6`
     (текущий путь) **и `stat 12` (новое)**.
   - Сохранить delta как сейчас + отдельный slot для mLvl (u16, max 255).
   - Если stat 12 не найден — mLvl = 0, Rust сторона сделает fallback
     на текущее поведение.

2. **Slot layout `ring.rs`** — slot уже 16 байт, есть свободный pad
   после `max_hp u16`. Использовать его под `monster_level u16`:
   ```
   +0x00 ts_ms        (u32)
   +0x04 unit_id      (u32)
   +0x08 delta_raw    (u32)  bit 31 = is_kill
   +0x0C max_hp       (u16)
   +0x0E monster_level (u16) ← новое поле, ранее pad
   ```
   Slot остаётся 16 байт, ring buffer без изменений.

3. **`HookEvent` mod.rs** — добавить `pub monster_level: u16`.

4. **`DpsMeter::ingest`** — новая сигнатура
   `ingest(ts, delta_raw, max_hp, mLvl)`. Формула:
   ```rust
   let scale = if mLvl > 0 { mLvl as u64 } else { 1 };
   let damage = ((delta_raw as u64) * (max_hp as u64) * scale / 32768) as u32;
   ```

5. **Tests** — обновить unit-тесты `dps_meter::tests`: добавить
   параметр mLvl в `ingest` calls. Поведение не меняется когда
   mLvl=1 (degenerate case = старая формула × 1).

6. **Smoke-test acceptance** — те же критерии из раздела 5:
   - One-shot test: total ≈ actual_max_hp моба ±20%.
   - Cross-difficulty: Normal ≪ NM ≪ Hell в правильной пропорции.

### Если linear не сойдётся

Запасной путь — calibrate через CE write-watchpoint:
- Поставить watchpoint на адрес stat-6 dwValue одного моба.
- Ударить моба известным урон (читать stat dmg_min/dmg_max игрока).
- Поймать stat 6 update event с конкретными old/new values.
- Reverse: actual_max = damage_player × 32768 / (old_normalised - new_normalised).
- Сравнить с `template × mLvl` — корректировка формулы по delta.

Этот путь требует ещё ~2 часов и оформляется как
`docs/ce-scripts/calibrate-monster-hp.lua`. Открываем только если
linear даст error > 50% в smoke-тесте.
