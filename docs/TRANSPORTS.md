# Маскировочные транспорты: фреймворк

> Статус: **исследовательский каркас**. Фреймворк реален и работает (реестр,
> валидация, интеграция с запуском движка). Ни один маскировочный транспорт,
> способный обходить белые списки, **не поставляется в рабочем виде** — и это
> осознанное честное ограничение, а не недоделка. Почему — в разделе
> [«Чего не хватает»](#чего-не-хватает-честно).

---

## 1. Зачем это существует

`zapret2-gui` умеет запускать `winws`/`winws2` с DPI-десинхронизацией
(`fake`/`multisplit`/`multidisorder` и т.д.). Это работает против «обычного»
ТСПУ, но умирает там, где сеть работает **по белым спискам**: корпоративные и
университетские сети, сегменты с полным TLS-фингерпринтингом, где DPI
пропускает только трафик «разрешённых» сервисов (Google, Discord, крупные CDN).

Техники для таких сетей — **маскировочные транспорты**: domain fronting,
мимикрия под разрешённые протоколы/сервисы, covert-каналы. У каждой техники
своя конфигурация, свои предусловия готовности (главное — **серверная
сторона**!) и свой способ встраивания в запуск движка. Без фреймворка каждая
экспериментальная техника превращается в ad-hoc хак в `supervisor.rs`: нет
единого контракта, нет uniform-валидации конфига, нет честного сигнала
«транспорт не может работать, и вот почему» — только молчание или падение
движка.

Фреймворк решает именно инженерную боль:

- единый контракт `Transport`: что такое транспорт, как он описывает себя,
  как проверяет готовность, как валидирует конфиг, как встраивается в запуск;
- реестр (`TransportRegistry`), который UI перечисляет через IPC — фронтенд
  **не хардкодит** ни id, ни названия, ни готовность транспортов;
- fail-closed поведение: транспорт, который не готов, **явно отказывает с
  причиной**, а не делает вид, что работает;
- JSON-схема конфигурации с понятными ошибками валидации.

Архитектурное вдохновение — идея pluggable transports (как у Tor) и подход
OpenFlux: транспорт описывается спекой, фабрика строит его по спеке,
протокол/маскировка — отдельная concern от движка запуска.

---

## 2. Архитектура

```
┌─────────────────────────────────────────────────────────────┐
│ UI: TransportsSection.tsx                                   │
│  invoke("list_transports") → TransportView[]                │
│  invoke("set_active_transport", { transport_id })           │
│  invoke("set_transport_config", { transport_id, config_json }) │
│    (persisting options; no UI editor for options yet —      │
│     researchers only for now)                               │
└──────────────────────────┬──────────────────────────────────┘
                           │ IPC (commands.rs)
┌──────────────────────────▼──────────────────────────────────┐
│ engine::transport                                           │
│  TransportRegistry (global, OnceLock, read-only после init)  │
│   ├─ DirectTransport            "direct"          [builtin]  │
│   └─ DomainFrontingTransport    "domain-fronting" [skeleton] │
│                                                             │
│  apply(id, config, args):                                   │
│    1. lookup → UnknownTransport{id, known}  (fail-closed)  │
│    2. config.enabled == false → passthrough (как direct):   │
│       availability gate пропускается — «выключен» значит    │
│       «выключен», отказывать нечему                        │
│    3. is_available() → Unavailable{reason}  (явный отказ)   │
│    4. validate_config() → InvalidConfig{message}            │
│    5. apply_args() → финальный Vec<String>                  │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│ Supervisor::start()                                         │
│  build_arguments()   ← существующая логика стратегий         │
│  apply_active_transport(args) ← НОВОЕ: поверх стратегий,    │
│                                 стратегию не меняет         │
│  spawn_process(winws2, final_args)                          │
└─────────────────────────────────────────────────────────────┘
```

### Ключевые типы (`src-tauri/src/engine/transport.rs`)

| Тип | Назначение |
|---|---|
| `trait Transport` | Контракт: `id()`, `name()`, `describe()`, `is_available()`, `validate_config()`, `apply_args()` |
| `TransportRegistry` | Регистрация, `get(id)`, `list()` (для UI), `apply()` (пайплайн запуска) |
| `TransportConfig` | `{ transport_id, enabled, options: JSON }` — сериализуемо, хранится в `AppSettings.transport_configs` |
| `TransportDescriptor` | Метаданные для UI: `experimental`, `requires_exit_infra`, `config_example` |
| `TransportAvailability` | `Available` / `Unavailable { reason }` |
| `TransportError` | `UnknownTransport` / `Unavailable` / `InvalidConfig` / `ApplyFailed` — все с текстом для пользователя |
| `TransportView` | Сериализуемый слепок для IPC: дескриптор + живая доступность |

### Правила честности (контракт, а не рекомендации)

1. `is_available()` обязан возвращать `Unavailable` с понятной причиной, пока
   не выполнено **любое** предусловие. Ложное `Available` — худший исход: у
   пользователя ложное чувство защищённости.
2. Любой транспорт с `requires_exit_infra = true` держит `is_available() =
   false`, пока автор явно не прикрутит health-check exit-инфраструктуры.
3. `apply_args()` — чистая трансформация списка аргументов; значения только из
   **провалидированного** конфига; никаких сырых пользовательских строк
   (путей, доменов) без allow-list санитизации — движок привилегированный.
4. Порядок в `apply()` зафиксирован: выключенный транспорт (`enabled: false`) —
   прозрачный passthrough **до** любых проверок («выключен» значит
   «выключен», отказывать нечему); дальше доступность → валидация →
   трансформация. Обходить порядок нельзя.
5. Неизвестный id в настройках → fail-closed с перечислением доступных и
   подсказкой вернуться к `direct`.

### Настройки (`win32/settings.rs`)

```jsonc
{
  "active_transport_id": null,        // null = "direct", легаси-запуск
  "transport_configs": {
    "domain-fronting": {
      "transport_id": "domain-fronting",
      "enabled": true,                // false = прозрачный passthrough (как direct),
                                      // availability gate пропускается: «выключен» значит «выключен»
      "options": { "front_domain": "...", "target_sni": "...", "timeout_ms": 8000 }
    }
  }
}
```

Новые поля — `Option` / `#[serde(default)]`: старые файлы настроек продолжают
загружаться, ничего не сбрасывается в дефолт.

### Локализация ошибок бэкенда (осознанное решение)

Строки `TransportError` — на русском и показываются в UI **как есть**
(verbatim через `formatErrorMessage`). Это не недоделка, а осознанное решение:

- кодовая база уже смешанная: `ProcessError` в `dev` — тоже en/ru, регрессии
  относительно существующей конвенции нет;
- динамические причины отказа (например, `reason` из `is_available()`)
  формируются бэкендом на лету — локализовать их можно только по кодам ошибок
  на фронте, чего пока нет ни для одного модуля;
- все *статичные* строки UI — в i18n (ru/en/uk), включая секцию транспортов.

Если динамические причины понадобятся на трёх языках — путь такой: коды
ошибок из бэкенда + ключи в `src/i18n/{ru,en,uk}.ts`. До тех пор — честный
русский текст вместо молчания.

### Сохранение конфига транспорта (IPC)

`set_active_transport` сохраняет только *выбор* транспорта. Для опций есть
отдельная команда `set_transport_config(transport_id, config_json)`:
парсит JSON, валидирует его схемой самого транспорта (`validate_config`) и
только потом пишет в `AppSettings.transport_configs` (флаг `enabled` при этом
сохраняется). Честное ограничение: UI-редактора опций пока нет — секция
«Транспорты» только выбирает транспорт, а `validate_transport_config`
существует для будущего построчного редактора. До его появления команда —
инструмент для исследователей.

---

## 3. Как написать свой транспорт (пошагово)

Пример: вы исследуете мимикрию под QUIC Google (`"quic-mimicry"`).

**Шаг 1.** Реализуйте трейт в `src-tauri/src/engine/transport.rs` (или новом
модуле, подключённом к реестру):

```rust
pub struct QuicMimicryTransport;

impl Transport for QuicMimicryTransport {
    fn id(&self) -> &'static str { "quic-mimicry" }
    fn name(&self) -> &'static str { "QUIC mimicry (research)" }

    fn describe(&self) -> TransportDescriptor {
        TransportDescriptor {
            id: self.id().into(),
            name: self.name().into(),
            description: "Мимикрия QUIC-трафика под googlevideo.com ...".into(),
            experimental: true,            // честно: исследование
            requires_exit_infra: true,     // если нужен сервер — true
            config_example: serde_json::json!({
                "mimic_domain": "googlevideo.com",
                "quic_version": "Q050"
            }),
        }
    }

    fn is_available(&self) -> TransportAvailability {
        // Пока нет проверенной серверной части — честный отказ:
        TransportAvailability::Unavailable {
            reason: "Нет серверной части: ...".into(),
        }
        // Когда появится: проверять доступность exit-ноды, валидность
        // сертификатов, версию helper-бинарника — и только тогда Available.
    }

    fn validate_config(&self, config: &TransportConfig) -> Result<(), TransportError> {
        // Разобрать config.options в типизированную структуру,
        // каждое поле — с понятным сообщением об ошибке.
        // Домены/порты — только через allow-list проверки
        // (см. is_plausible_domain в domain-fronting).
    }

    fn apply_args(&self, config: &TransportConfig,
                  args: Vec<String>) -> Result<Vec<String>, TransportError> {
        // Чистая трансформация: добавить/изменить флаги winws2
        // ТОЛЬКО из провалидированного конфига.
    }
}
```

**Шаг 2.** Зарегистрируйте в `TransportRegistry::new()`:

```rust
registry.register(Box::new(QuicMimicryTransport));
```

Дубли id игнорируются (первый побеждает) — встроенный транспорт нельзя
случайно затенить.

**Шаг 3.** Проверьте:

- `cargo test transport` — юнит-тесты реестра и валидации;
- IPC `list_transports` — транспорт виден в UI с бейджами;
- IPC `set_active_transport` с вашим id — при `Unavailable` UI показывает
  причину, выбор не сохраняется;
- запуск движка с недоступным транспортом — движок **не стартует**, в логе
  точная причина.

**Шаг 4.** Добавьте локализацию названия/описания в UI: в
`src/components/TransportsSection.tsx` (функции `transportName` /
`transportDesc`) + ключи `name_<id>` / `desc_<id>` в
`src/i18n/{ru,en,uk}.ts` и `src/i18n/types.ts`.

**Шаг 5.** Опишите транспорт в этом документе: что маскирует, что нужно на
сервере, как проверить готовность.

---

## 4. Референс: каркас domain-fronting транспорта

`DomainFrontingTransport` (`"domain-fronting"`) — структурный референс:

- трейт реализован полностью: id, дескриптор (`experimental: true`,
  `requires_exit_infra: true`), схема конфига;
- `validate_config()` строгая и понятная: `front_domain` / `target_sni` —
  правдоподобные домены, не равны друг другу (иначе маскировка бессмысленна),
  `timeout_ms` в диапазоне 100–30000;
- `is_available()` **всегда** возвращает `Unavailable` с причиной «нет
  серверной части»;
- `apply_args()` отказывает второй линией обороны (на случай обхода гейта);
- код помечен `TODO` для исследователя: что сделать, чтобы транспорт ожил.

Пример конфига: `docs/examples/domain-fronting.example.json`.

---

## 5. Что НУЖНО для реальной маскировки (по мотивам OpenFlux)

Чтобы транспорт класса «маскировка под разрешённые сервисы» заработал
по-настоящему, недостаточно клиентского кода. Нужно:

1. **Domain fronting — cooperating endpoint.** TLS-хендшейк с SNI
   `allowed-cdn.example.com`, а реальное назначение — внутри зашифрованного
   канала (HTTP Host / inner SNI). Это требует CDN/прокси, который (а)
   терминирует TLS валидным сертификатом front-домена и (б) маршрутизирует по
   внутреннему назначению. Без своей инфраструктуры — не работает; публичные
   CDN массово отключили fronting именно как abuse-вектор.
2. **Mimicry — исследования протоколов.** Мимикрия под QUIC/Discord/Telegram:
   нужны актуальные слепки хендшейков разрешённых сервисов, их TLS-фингерпринты
   (JA3/JA4), поведение при retransmit, тайминги. Это исследовательская работа
   с захватом трафика, а не «флаг в конфиге». Фингерпринт, снятый полгода
   назад, палится DPI.
3. **Exit-инфраструктура.** Любая маскировка заканчивается вопросом «а дальше
   куда»: нужен exit — нода, которая принимает замаскированный трафик и
   выпускает его в интернет. Это сервер(а), канал, мониторинг, ротация при
   блокировке. Без exit-ноды маскировочный транспорт — театр.
4. **Путь данных, а не только аргументы.** winws2-десинхронизация не умеет
   делать fronting сама по себе: скорее всего нужен локальный helper-процесс
   (TLS-forwarder), запускаемый *до* winws2, с подстройкой `--wf-*` фильтров.
   Фреймворк сейчас трансформирует только аргументы winws2 — для настоящего
   транспорта понадобится расширение (хук «подготовить окружение»).
5. **Операционная безопасность.** Маскировка, которой пользуется тысяча
   человек с одинаковым фингерпринтом, детектится за дни. Нужны рандомизация,
   ротация front-доменов, телеметрия блокировок.

---

## 6. Чего не хватает (честно)

| Нужно | Статус |
|---|---|
| Фреймворк: трейт, реестр, валидация, IPC, UI | ✅ есть |
| Сохранение опций транспорта (`set_transport_config`, валидация по схеме) | ✅ есть (IPC; UI-редактора опций пока нет — честное ограничение) |
| Референсный каркас domain-fronting | ✅ есть (честно недоступен) |
| Exit-нода / fronting-endpoint | ❌ нет, и не планируется в этом репозитории |
| Исследования мимикрии (слепки, фингерпринты) | ❌ нет |
| Серверная часть любого транспорта | ❌ нет |
| Helper-процессы в пайплайне запуска | ❌ фреймворк пока трансформирует только аргументы |
| Рабочий обход белых списков «из коробки» | ❌ **не даёт и не обещает** |

---

## 7. Самокритика: почему фреймворк, а не готовый прорыв

**Фреймворк, а не прорыв, потому что прорыв — это не код на клиенте.**
Настоящий обход белых списков (как у OpenFlux) — это система: клиентская
маскировка + серверный exit + исследования протоколов + операционка
(ротация, телеметрия). Клиентский код без серверной части — это декларация о
намерениях. Мы могли бы написать транспорт, который «включается» и ничего не
делает (или, хуже, светит реальный SNI) — это было бы враньё пользователю
ради галочки «фича готова». Вместо этого фреймворк честно разделяет:
инфраструктура для экспериментов — есть; волшебной кнопки — нет.

**Что остаётся за пользователем/исследователем, чтобы транспорт заработал
по-настоящему:**

1. Развернуть или найти cooperating exit-инфраструктуру (свои серверы, свой
   fronting-endpoint) — проект её не поставляет и не будет.
2. Провести исследования маскировки: снять актуальные хендшейки/фингерпринты
   разрешённых сервисов, проверить их против целевого DPI.
3. Реализовать `is_available()` как настоящий health-check своей
   инфраструктуры, а не константу.
4. Спроектировать путь данных (возможно, helper-процесс до winws2) и расширить
   фреймворк хуком подготовки окружения — сейчас его нет, это известное
   ограничение.
5. Взять на себя операционку: ротация, мониторинг блокировок, обновление
   слепков.

Фреймворк — это взлётная полоса. Самолёт (инфраструктура, исследования,
серверная часть) — за тем, кто хочет лететь.

---

## 8. English summary

**Camouflage transports: framework (research scaffold).**

DPI desync tricks die on allow-listed networks. Camouflage transports (domain
fronting, protocol mimicry) are the research direction — but every technique
needs its own config, readiness preconditions (above all: **server-side exit
infrastructure**), and a way to plug into the engine launch pipeline.

This ships the **framework**, not the breakthrough:

- `Transport` trait (`id`, `describe`, `is_available`, `validate_config`,
  `apply_args`) + `TransportRegistry` with a fail-closed apply pipeline
  (lookup → availability gate → config validation → arg transformation);
- IPC (`list_transports`, `set_active_transport`, …) and a UI section
  «Transports (experimental)» that renders the registry snapshot — the
  frontend hardcodes nothing;
- honest refusal: an unavailable transport fails with an explicit reason,
  never silently pretends to work;
- one reference skeleton (`domain-fronting`) that compiles and registers but
  reports `Unavailable` ("no server side") by design.

**What it does NOT give you out of the box:** working allow-list
circumvention. That requires exit infrastructure, mimicry research, a server
side, and operations (rotation, telemetry) — none of which this project
ships. The framework is the runway; the plane is up to the researcher
(see §5–§7).
