# Дизайн: интеграция Variant A (Studio) — визуал и поведение `cm`

Дата: 2026-05-27
Статус: согласован, готов к плану реализации
Подпроект 1 из 2 (Подпроект 2 — attached overlay — отдельный цикл)
Источник дизайна: handoff-бандл Claude Design, Variant A · Studio
(`design_handoff_cm/README.md`, `studio.jsx`, `terminal.jsx`)

## Цель

Переписать визуал существующего `cm` под hi-fi дизайн Variant A: тёплая тёмная
палитра с янтарным акцентом, header со статусами, нижний footer хоткеев,
раскладка 40/60, многострочные карточки сессий, живой braille-спиннер, цветной
preview, рестайл всех модалок + сегментный селектор агента, fuzzy-фильтр, g/G,
пустое состояние и экран ошибки «tmux not found».

Сохраняем существующую архитектуру (tmux — источник правды, stateless на диске,
`@cm_managed`/`@cm_agent` user options, IO-free `App` + `Action`). НЕ входит в
этот подпроект: overlay «внутри сессии» (Ctrl-Q, стриминг capture-pane) — это
Подпроект 2; до тех пор attach остаётся как сейчас (выход из TUI → `tmux
attach` → возврат по Ctrl-B D).

## Согласованные технические решения

1. **Git — только чтение, через `git` CLI** (`Command`, без Rust-зависимости).
   На каждом `refresh` для `dir` сессии: ветка = `git -C <dir> symbolic-ref
   --short HEAD`; diff = `git -C <dir> diff --shortstat`. Не-репо → `None`,
   `⎇`/diff не показываются.
2. **Спиннер без `tokio`:** синхронный loop, poll-таймаут ~80 мс; кадр braille
   считается из `Instant` (`elapsed_ms / 80 % 10`); тяжёлый `refresh()` —
   throttle по `refresh_interval_ms`.
3. **Два статуса** (не три): `Running` (живой спиннер, amberHi) и `Idle` (`·`,
   dim). «waiting» по diff'у `capture-pane` надёжно не определить — опускаем.
   В header — счётчики `running`/`idle`.
4. **Часы/возраст без зависимостей:** `age` = `now − session_created` →
   `humanize_age`; часы `HH:MM` = вызов `date +%H:%M` (локальная tz).
5. **Новые зависимости:** только `ansi-to-tui` (цветной preview). Без `tokio`,
   `chrono`, `directories`, `git2`; путь к конфигу — через `$HOME`.

## Палитра (`src/theme.rs`)

```rust
use ratatui::style::Color;
pub const BG:        Color = Color::Rgb(0x1a, 0x17, 0x14);
pub const BG_RAISED: Color = Color::Rgb(0x21, 0x1d, 0x18);
pub const BG_SUNKEN: Color = Color::Rgb(0x16, 0x13, 0x0f);
pub const SEL_BG:    Color = Color::Rgb(0x2a, 0x1d, 0x18); // заранее смешанный amberDim (альфы в терминале нет)
pub const TEXT:      Color = Color::Rgb(0xe8, 0xdf, 0xd1);
pub const TEXT_BOLD: Color = Color::Rgb(0xf4, 0xec, 0xdd);
pub const MUTED:     Color = Color::Rgb(0x8a, 0x7f, 0x6e);
pub const DIM:       Color = Color::Rgb(0x5c, 0x54, 0x4a);
pub const BORDER:    Color = Color::Rgb(0x2f, 0x2a, 0x23);
pub const BORDER_HI: Color = Color::Rgb(0x40, 0x3a, 0x30);
pub const AMBER:     Color = Color::Rgb(0xd9, 0x77, 0x57);
pub const AMBER_HI:  Color = Color::Rgb(0xf4, 0xa3, 0x6a);
pub const GREEN:     Color = Color::Rgb(0x7a, 0xb8, 0x7a);
pub const RED:       Color = Color::Rgb(0xc7, 0x5d, 0x4a);
pub const YELLOW:    Color = Color::Rgb(0xd6, 0xb2, 0x5f);
pub const BLUE:      Color = Color::Rgb(0x6a, 0x9f, 0xb5);
```
Глифы (константы там же): `◆` лого, `▸` заголовок preview, `✻` агент, `▍`
полоса выделения, `·` idle, `⎇` ветка, `━` rule, `│` разделитель header,
`＋` new, `✕` kill, `?` help.

## Структура модулей

```
src/
├── theme.rs        (нов.) палитра + глифы
├── spinner.rs      (нов.) BRAILLE: [&str;10]; frame_at(elapsed_ms:u128)->&str  (чистая)
├── git.rs          (нов.) GitInfo{branch,added,removed}; read(dir)->Option<GitInfo>
├── timeutil.rs     (нов.) humanize_age(secs:i64)->String; clock_hhmm()->String
├── app.rs          (изм.) Status::Idle, filter, agent-селектор в CreateForm, spinner_frame, now_unix
├── tmux.rs         (изм.) capture_pane → `-e`; Session += git: Option<GitInfo>
├── browse.rs       (без изм.)
├── config.rs       (без изм.)
├── main.rs         (изм.) tick 80мс, throttled refresh, фильтр/g/G клавиши, экран ошибки tmux в TUI
└── ui/             (замена ui.rs)
    ├── mod.rs      draw(f,&App) + раскладка + диспатч режимов/модалок + centered()
    ├── header.rs   render_header(f, area, &App, spinner_frame)
    ├── footer.rs   render_footer(f, area, items)
    ├── sessions.rs render_sessions(f, area, &App, spinner_frame)
    ├── preview.rs  render_preview(f, area, &App)
    ├── modal_new.rs / modal_kill.rs / modal_help.rs
    ├── empty.rs
    └── error.rs
```

## Модель данных

```rust
// tmux.rs
pub enum Status { Running, Idle }              // было Running/Waiting
pub struct Session {
    pub name: String, pub dir: String, pub created: i64,
    pub agent: String, pub status: Status, pub attached: bool,
    pub git: Option<GitInfo>,                  // нов.
}
// git.rs
pub struct GitInfo { pub branch: String, pub added: u32, pub removed: u32 }
pub fn read(dir: &str) -> Option<GitInfo>;     // None если не git-репо
// app.rs (App += )
pub filter: Option<String>,      // Some => режим фильтра активен
pub spinner_frame: usize,        // ставит main каждый тик
pub now_unix: i64,               // ставит refresh (для age)
// app.rs CreateForm (+= )
pub agent_choices: Vec<String>,  // config.agent_presets + "custom…"
pub agent_index: usize,          // выбранный
// (поле dir-picker'а из прошлого подпроекта сохраняется)
```
`compute_status`: изменился → `Running`, иначе `Idle` (первое наблюдение — `Idle`).

## Event loop (main.rs)

```
poll_timeout = min(80ms, refresh_interval)
loop {
  app.spinner_frame = (start.elapsed().as_millis() / 80 % 10) as usize
  draw(app)
  if poll(poll_timeout)? { handle key (Press) } else { /* просто тик спиннера */ }
  if last_refresh.elapsed() >= refresh_interval { app.refresh(); last_refresh = now }
  if should_quit { break }
}
```
- tmux нет в PATH → `App` стартует в режиме `Mode::Error` (или флаг), `ui/error.rs`,
  выход по `q`. (Раньше — `eprintln!`+`exit(1)` до TUI; теперь — экран в TUI.)
- Новые клавиши (режим List): `/` → `Mode`-фильтр (печать пополняет `filter`,
  Backspace, Esc — сброс в None); `g` → первая, `G` → последняя сессия.
  Существующие клавиши без изменений.
- `refresh()` дополнительно: ставит `now_unix`, для каждой сессии вызывает
  `git::read(&dir)` → `session.git`. Статусы — как раньше (diff capture).

## Фильтрация

`/` включает фильтр. Видимый список = сессии, чьё `name` содержит подстроку
`filter` (регистронезависимо; простой substring, не настоящий fuzzy — YAGNI).
Навигация/выбор работают по отфильтрованному виду; `selected` клампится.
Header и footer показывают активный фильтр (`/foo`). Esc → `filter=None`.

## Разбор по экранам

**Main.** Вертикаль `[Length(2) header, Min(0) body, Length(2) footer]`. Header:
строка спанов (`◆ cm` amber·bold · «claude · session manager» dim · `│` dim ·
«N sessions» · `<спиннер> R running` amberHi · `· I idle` dim · справа `HH:MM`
muted) + строка `━` BORDER. Body: `[Percentage(40) sessions, Length(1) │,
Min(0) preview]`. Footer: строка `━`-сверху (Block borders TOP) + строка
хоткеев (`n new` amber·bold, далее textBold ключ + muted подпись).

**Карточка сессии** (sessions.rs) — `List` с multi-line `ListItem` (3 строки),
`highlight_symbol = "▍ "` (amber), `highlight_style` = bg `SEL_BG` (выделенная
строка), имя выделенной — bold AMBER_HI:
- стр1: `name` (TEXT_BOLD; выделенная — AMBER_HI bold) … справа статус:
  `Running` → `<spinner_frame> running` (AMBER_HI), `Idle` → `· idle` (MUTED).
- стр2: `dir` (MUTED).
- стр3: `✻ <agent>` (✻ AMBER если выделена иначе DIM; имя агента MUTED) ·
  `⎇ <branch>` (DIM) · `+<added> −<removed>` (GREEN/RED) … справа `<age>` (MUTED).
  Если `git == None` — `⎇`/diff не рендерятся.

**Preview** (preview.rs): `▸ <name>` (AMBER `▸`, TEXT_BOLD bold) … справа `<age>`
(DIM); строка `<path> · ⎇ <branch>` (MUTED/DIM, ветка только если репо); `━`;
контент = `tmux capture-pane -p -e -t <name>` → `ansi_to_tui::IntoText` →
`Text` → `Paragraph` (wrap). Если capture пуст — пусто.

**New session** (modal_new.rs): нижние слои рисуются приглушённо (весь fg в DIM —
имитация затемнения; альфы/блюра в терминале нет). `Clear` + Block (рамка
AMBER, bg BG_RAISED). Заголовок `＋ New session` (AMBER bold) … `N of N` (dim).
Поля:
- **Name** — лейбл капсом (MUTED), значение в «врезке» (bg BG_SUNKEN, левый бортик
  BORDER_HI), курсор `frame.set_cursor` если поле в фокусе.
- **Directory** — то же + строка `✓ exists` (GREEN ✓) и `⎇ <branch>` если репо
  (переиспользуем dir-picker из прошлого подпроекта: список подпапок остаётся).
- **Agent** — лейбл AMBER; сегментный селектор `claude · codex · aider · gemini
  · custom…` (выбранный сегмент — bg AMBER, fg BG; `←/→` циклят `agent_index`);
  строка `$ <resolved-команда>` (BG_SUNKEN) + справа «resolved command» (dim);
  строка «Claude Code — found at <path>» (`command -v <bin>`; если не найдено —
  «not found in PATH» YELLOW).
- Снизу: `━`; «will run `tmux new -s <name> -c <dir> "<команда>"`» (muted/text).
Footer (режим Create): `↵ create · ⇥ next field · ←→ pick agent · esc cancel`.

**Kill** (modal_kill.rs): приглушённый фон + `Clear` + Block (рамка RED). `✕ Kill
session?` (RED ✕ в кружке-имитации, TEXT_BOLD), строка `<name>` (AMBER) `· <status>`,
`<path> · ⎇ <branch>` (MUTED), предупреждение (DIM), «кнопки» `y · yes, kill`
(bg RED) и `n · no` (рамка BORDER), `esc to dismiss`. Footer: `y yes,kill · n no
· esc cancel`.

**Help** (modal_help.rs): header + 4 группы в 2 колонки (grid):
- Navigation: `↑↓`/`k j` move · `g G` first·last · `/` filter
- Session: `↵` attach · `n` new · `d` kill · `r` rename
- Preview: (в этом подпроекте — статично, поясняет автообновление)
- App: `?` help · `q` quit (sessions stay)
Ключи AMBER_HI, подписи TEXT. Footer: `esc close · q quit`.
(Пункты `c duplicate`, `space pause`, `p pin`, `shift+r`, `ctrl-r reload` из
дизайна — НЕ реализуются в v1; в help не показываем то, чего нет.)

**Empty** (empty.rs): по центру крупный `◆` (AMBER), «No sessions yet» (TEXT_BOLD
bold), подпись (MUTED), `[n] start your first session`, `━ tip ━`, подсказка.
Footer: `n new session · ? help · q quit`.

**Error: tmux missing** (error.rs): `◆`, «tmux not found in PATH» (RED/TEXT_BOLD),
инструкции: macOS `brew install tmux`, Ubuntu `apt install tmux`, Arch `pacman
-S tmux`. Выход по `q`.

## Хоткеи (итог для Подпроекта 1)

| Клавиша | Контекст | Действие |
|---|---|---|
| `k`/`j` (+стрелки) | список | навигация |
| `g` / `G` | список | первая / последняя |
| `/` | список | фильтр (Esc — сброс) |
| `Enter` / `o` | список | attach (как сейчас) |
| `n` | список | новая сессия |
| `d` → `y/n` | список | kill |
| `r` | список | rename inline |
| `←/→` | new-modal | выбор агента |
| `?` | везде | help |
| `q` | список/help/error | выход |
| `Esc` | модалка/фильтр/rename/help | отмена/закрыть |

`Ctrl-Q` и overlay — Подпроект 2, здесь не реализуются.

## Обработка ошибок

- Нет `tmux` → экран `error.rs` в TUI, выход по `q` (ненулевой код при выходе).
- `git`/`date`/`command -v` недоступны или падают → graceful: git → `None`,
  часы → пусто, resolved-путь → «not found». Никаких паник.
- `capture-pane`/`ansi-to-tui` ошибка → пустой preview (текст не валит UI).
- tmux-команды (kill/rename/new) с ошибкой → строка в footer (как сейчас).

## Тестирование

- **Unit (чистые):** `spinner::frame_at` (границы 0/79/80/799/800мс → кадры),
  `timeutil::humanize_age` (сек→«Ns/Nm/Nh/Nd»), `git`-парсинг `--shortstat`
  («3 files changed, 12 insertions(+), 4 deletions(-)» → (12,4); без insertions;
  без deletions; пустой → (0,0)), `compute_status` (Idle), fuzzy/substring
  фильтр списка, agent-селектор `←/→` (цикл по `agent_choices`).
- **Интеграция (temp-dir):** `git::read` на временном `git init` репо (ветка,
  diff после правки файла) и на не-репо (→ None).
- **UI snapshot (TestBackend):** для детерминизма часы/спиннер замоканы (передаём
  фикс. `spinner_frame`, clock через инъекцию строки). Рендер: header
  (счётчики/часы), карточка (running кадр 0 со спиннером; idle; с git и без;
  выделенная с `▍`), preview, modal_new (3 поля + селектор), modal_kill,
  modal_help, empty, error. Проверяем наличие глифов/строк/ключевых спанов.

## Вне scope (Подпроект 1)

- Overlay «внутри сессии», `Ctrl-Q`, стриминг/проброс ввода (Подпроект 2).
- Третий статус `waiting`/`◐`.
- `c duplicate`, `space`/`p` pin-pause preview, `shift+r`, `ctrl-r reload`.
- Любые git-операции кроме чтения branch/diff.
- Настоящий fuzzy-матчинг (делаем substring).

## Критерии готовности

- [ ] `cargo build --release` без warning'ов; `clippy -D warnings` чист.
- [ ] Тёплая палитра применена везде; глифы из дизайна на месте.
- [ ] Header со счётчиками running/idle, часами и живым спиннером.
- [ ] Footer прижат к низу; preview занимает высоту.
- [ ] Раскладка 40/60; многострочные карточки с полосой выделения, агентом,
      git (если репо), возрастом.
- [ ] Спиннер крутится ~80мс/кадр; refresh — по `refresh_interval_ms`.
- [ ] Preview цветной (ansi-to-tui).
- [ ] New/kill/help/empty/error экраны соответствуют дизайну; селектор агента
      с резолвом команды; `←/→` работают.
- [ ] `/` фильтр, `g`/`G` навигация работают.
- [ ] Запуск из любой директории; сессии переживают выход; конфиг в `~/.claude-manager`.
- [ ] Все тесты зелёные.
