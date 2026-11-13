# zapret2 Desktop Supervisor & GUI — Архитектура проекта

Настоящий документ описывает организацию каталогов и архитектуру **zapret2 Desktop
Supervisor & GUI**: двухбинарную схему (GUI + фоновая служба), IPC между ними,
владение процессами движка и гарантированное поведение при выходе.

---

## 🧩 Два бинарника, а не один

Приложение собирается из **двух** исполняемых файлов (Cargo-пакет `zapret2-supervisor`):

| Бинарник | Исходник | Роль |
|---|---|---|
| `zapret2.exe` | `src-tauri/src/main.rs` (Tauri, `productName: "zapret2"`) | GUI: окно, трей, команды, автозапуск. |
| `zapret2-svc.exe` | `src-tauri/src/bin/svc.rs` (`[[bin]] name = "zapret2-svc"`) | Headless-служба: владеет движком и bridge, переживает закрытие GUI. |

Служба — не «дубликат GUI», а отдельный headless-процесс. GUI может работать в двух
режимах (см. ниже): сам держать движок (**LOCAL**) либо подключиться к службе
(**REMOTE**). В REMOTE именно служба владеет `winws`/`winws2` и Telegram-bridge,
поэтому обход продолжает работать, когда окно закрыто или GUI упал.

---

## 📁 Структура каталогов

```text
zapret2-gui/
├── bin/                          # Авторитетные бинарники и полезные нагрузки
│   ├── winws2.exe                # Движок DPI bypass (bol-van zapret2 v2)
│   ├── winws.exe                 # Движок первого поколения (Flowseal zapret v1)
│   ├── WinDivert.dll             # Пользовательская библиотека перехвата пакетов WFP
│   ├── WinDivert64.sys           # Драйвер ядра WinDivert x64
│   ├── cygwin1.dll               # POSIX runtime для winws/winws2
│   ├── *.bin                     # Дампы ClientHello / QUIC Initial / STUN
│   └── tgproxy/                  # Telegram MTProto→WebSocket bridge (tg-ws-proxy)
│
├── lists/                        # Списки доменов, IP и исключений
├── strategies/                   # Lua-стратегии zapret2 (antidpi, auto, obfs)
│
├── src/                          # Frontend (React 19, TypeScript, Tailwind, Vite)
│   ├── components/               # UI (Dashboard, Strategies, Logs, ...)
│   ├── i18n/                     # Локализация интерфейса (ru, en, uk)
│   ├── types/, utils/, assets/
│   ├── App.tsx                   # Корневой компонент
│   └── main.tsx                  # Точка входа React
│
├── src-tauri/                    # Backend (Rust, Tauri v2)
│   ├── src/
│   │   ├── main.rs               # GUI-бинарник (zapret2.exe): окно, трей, автозапуск
│   │   ├── bin/svc.rs            # Headless-бинарник (zapret2-svc.exe)
│   │   ├── commands.rs           # Tauri IPC-команды (supervisor, autostart, dns, exit, ...)
│   │   ├── svc_ipc.rs            # Протокол svc↔GUI: pipe/socket, JSON, версии, сообщения
│   │   ├── svc_server.rs         # Сторона svc: pipe-сервер, обработка команд, shutdown
│   │   ├── svc_client.rs         # Сторона GUI: connect, spawn svc, миграция LOCAL→REMOTE
│   │   ├── engine/               # Ядро супервизора
│   │   │   ├── supervisor.rs     # Жизненный цикл движка (start/stop), Job Object
│   │   │   ├── tgproxy.rs        # Обёртка Telegram-bridge (запуск, конфиг, режимы)
│   │   │   ├── transport.rs      # Трейт транспортов + реестр
│   │   │   ├── events.rs         # EventSink — события для UI
│   │   │   ├── logger.rs         # Кольцевой буфер логов
│   │   │   ├── probes.rs         # Диагностика доступности
│   │   │   ├── speedtest.rs, managed.rs, block_classifier.rs
│   │   │   └── mod.rs
│   │   └── platform/             # ОС-специфичная интеграция
│   │       ├── windows/          # Win32: job_object, process, privileges, dns,
│   │       │                     #   autostart, updater, app_split, smart_hosts, ...
│   │       └── linux/            # /proc, сигналы, unix socket, systemd
│   ├── vendor/tg-ws-proxy/       # Вендорный Telegram-bridge (исходники Rust)
│   ├── capabilities/             # Права доступа Tauri v2
│   ├── icons/
│   ├── app.manifest              # UAC-манифест requireAdministrator (RT_MANIFEST)
│   ├── Cargo.toml                # [[bin]] zapret2-svc + основной GUI-бинарник
│   └── tauri.conf.json           # productName "zapret2", окно, сборка
│
├── scripts/                      # setup_binaries.ps1/.sh, linux_smoke.py
├── docs/                         # Техническая документация (этот файл и др.)
├── packaging/                    # Сборка установщиков
├── public/, assets/              # Статика frontend
├── CONFIG.md, CONFIG_GUIDE.md    # Правила задач / руководство по параметрам
├── README.md, README_EN.md       # Описание проекта (ru / en)
└── LICENSE                       # MIT
```

> Сборочные артефакты (`portable/`, `dist/`, `node_modules/`, `src-tauri/target/`)
> в git не хранятся. `portable/` — собранная раскладка приложения, которую
> формирует `scripts/make_portable.ps1`.

---

## ⚙️ Режимы работы: LOCAL и REMOTE

Backend в GUI (`SupervisorBackend`) бывает двух видов:

- **LOCAL** — GUI сам запускает `winws`/`winws2` и держит их в собственном
  in-process Job Object. Это стартовый режим по умолчанию.
- **REMOTE** — GUI подключается к уже работающей (или только что запущенной)
  службе `zapret2-svc.exe` по IPC. Движком и bridge владеет служба; GUI лишь
  отправляет команды и получает события.

Переходы:

1. **Обычный запуск** — GUI стартует в LOCAL и подключается к службе, только если
   она **уже** работает (сам службу не spawn-ит).
2. **Автозапуск** (`--minimized` / `--autostart`) — после короткой задержки GUI
   проверяет `backend_needs_attach` (LOCAL + найден `zapret2-svc.exe`), вызывает
   `ensure_svc_attached` (spawn службы + ожидание pipe до 10 с), переключается в
   REMOTE и стартует обход. Bridge мигрирует в службу вместе с движком.
3. **Выход (Quit)** — гарантированно останавливает и службу, и движок, и драйвер
   (см. «Поведение выхода»).

`find_svc_exe` ищет `zapret2-svc.exe` рядом с `current_exe` или в `resources/`.

---

## 🔌 IPC: служба ↔ GUI

Транспорт и формат (`svc_ipc.rs`):

- **Windows:** named pipe `\\.\pipe\zapret2-svc`.
- **Linux:** unix socket `/run/zapret2/svc.sock` (systemd `RuntimeDirectory`);
  путь переопределяется переменной `ZAPRET2_SVC_SOCKET`. После bind служба
  выставляет socket `0660` — аналог DACL для pipe.
- **Формат:** newline-delimited JSON. Каждая строка — один `ClientMessage`
  (GUI → svc) или `ServerMessage` (svc → GUI).
- **Версия протокола:** `PROTOCOL_VERSION = 2`. Handshake `Hello` отвергает
  несовместимых пиров. v2 добавил команды `tgproxy-*`: bridge переехал из GUI в
  службу, чтобы переживать падения GUI так же, как движок.
- **Single-instance:** именованный mutex `Global\zapret2-svc-single-instance`.

Команды GUI (`ClientMessage`, `cmd` в kebab-case): `hello`, `get-status`, `start`,
`stop`, `get-logs`, `clear-logs`, `subscribe`, `shutdown`, а также `tgproxy-start`,
`tgproxy-stop`, `tgproxy-get-status`, `tgproxy-set-config`,
`tgproxy-regenerate-secret`, `tgproxy-set-mode`, `tgproxy-test-connection`,
`tgproxy-set-lan`.

Ответы/события службы (`ServerMessage`): `welcome` (версия + снапшот статуса),
`response` (`ok`/`error`/снапшоты), `event`. События (`SvcEvent`) зеркалят
`EventSink` один-к-одному: `log-batch`, `state-changed`, `probe-update`,
`logs-cleared`, `tgproxy-state`.

**Безопасность.** Pipe/socket — только локальная машина. Любой локальный процесс
может *наблюдать* состояние; *мутирующие* команды дополнительно gated админ-проверкой
самого супервизора (`Supervisor::start` отказывает без elevation). Это соответствует
модели угроз одно-пользовательской рабочей станции. DACL pipe (Windows):
`D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)`.

### Создание pipe (важная деталь)

`create_secure_pipe` создаёт pipe с `PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED`.
Флаг **OVERLAPPED обязателен**: tokio регистрирует handle в I/O completion port
(`PollEvented`), а синхронный pipe IOCP отвергает с `ERROR_INVALID_PARAMETER`
(os error 87) — без флага сервер никогда не принял бы клиента. Ошибка
`GetLastError` снимается **до** `LocalFree`, чтобы тот её не затёр.

### Spawn службы

`spawn_svc_detached` запускает службу с `DETACHED_PROCESS | CREATE_NO_WINDOW` и
предпочитает `CREATE_BREAKAWAY_FROM_JOB`, чтобы служба пережила любой Job Object,
в котором сидит GUI. Если GUI находится в job, запрещающем breakaway,
`CreateProcess` падает с `ERROR_ACCESS_DENIED` (os error 5) — тогда spawn
повторяется **без** флага breakaway. Служба всё равно стартует и держит свой
собственный Job Object (`KILL_ON_JOB_CLOSE`) для движка.

---

## 🪟 Job Object и владение процессами

Все дочерние процессы движка привязаны к Windows Job Object с флагом
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: когда handle job закрывается (или
процесс-владелец умирает), ядро принудительно завершает всех членов job. Это
исключает зомби-`winws` и зависший драйвер WinDivert после закрытия владельца.

- В **LOCAL** job принадлежит GUI.
- В **REMOTE** job принадлежит службе. Завершение службы закрывает её job и
  уносит `winws` с собой — именно на этом построен force-kill fallback при выходе.

---

## 🚪 Поведение выхода

- **Кнопка X / запрос закрытия окна** — сворачивание в трей **по дизайну**. Обход
  продолжает работать (в REMOTE его держит служба). Это не завершение приложения.
- **Явный Quit («Выход» из трея)** — `graceful_exit` гарантированно останавливает
  всё, чтобы не осталось `zapret2-svc` + `winws`, фильтрующих трафик без GUI:
  1. В LOCAL — останавливает in-process bridge и движок.
  2. **Graceful shutdown службы, ограниченный 4 с.** Используется подключённый
     клиент, либо попытка connect (покрывает Local-GUI, пока служба от прошлой
     сессии ещё жива). Таймаут нужен потому, что «заштормованный» runtime службы
     может перестать принимать соединения, и команда shutdown никогда не дойдёт.
  3. **Force-kill fallback.** По таймауте (или если pipe wedged) GUI принудительно
     завершает образ службы (`kill_stray_svc_processes`) — её Job Object уносит
     `winws`; затем выметает любые stray-движки (`kill_stray_winws2_processes`).
     Обе функции перепроверяют образ процесса перед kill (TOCTOU-guard).
  4. **Пауза ~300 мс**, чтобы завершённый движок освободил handle WinDivert, затем
     `cleanup_windivert_driver` останавливает драйвер. (Останавливать драйвер под
     живым `winws` нельзя — это blackhole-ит трафик.)
  5. `app.exit(0)`.

Fallback идемпотентен: если служба уже чисто завершилась по graceful-команде,
убивать нечего. Linux-форма `kill_stray_svc_processes` работает через `/proc` +
`SIGKILL`; root-овую службу под systemd останавливает service manager.

---

## 🌉 Telegram-bridge (tgproxy) и cooldown circuit-breaker

Bridge (MTProto → WebSocket) живёт **в службе** (`engine/tgproxy.rs` — обёртка;
сам прокси — вендорный `src-tauri/vendor/tg-ws-proxy/`). Режимы соединения:
`auto`, `cfproxy`, `direct`. В `auto`/`cfproxy` используется tier Cloudflare-реле
(community CF domain list), затем raw-TCP last-resort к DC IP.

**Проблема шторма.** Когда CF-реле недостижимы, а DC IP SYN-блокированы, каждое
клиентское соединение заново обходило все реле и заново платило каждый connect
timeout, затем срывалось в raw-TCP и платило ещё и его timeout. При множестве
клиентов это насыщает движок, и bridge сам не восстанавливается.

**Cooldown circuit-breaker** (per-target память отказов на `CooldownMap`):

- `CF_DOMAIN_FAIL` — ключ по **базовому домену реле** (без префикса `kws{N}.`),
  общий для всех DC: реле, которое лежит, лежит для каждого DC. Реле, провалившее
  TLS/upgrade или timeout, пропускается на `cf_fail_cooldown`. **Redirect очищает**
  cooldown (реле ответило — Telegram просто увёл WS с него, это не транспортный
  отказ).
- `TCP_FALLBACK_FAIL` — ключ по **DC IP**, передышка `TCP_FALLBACK_COOLDOWN = 30 с`.
  SYN-блокированный IP fast-drop-ится вместо повторной оплаты connect timeout на
  каждом клиенте.

Оба cooldown истекают по времени и re-probe-ят цель, поэтому реле или DC, ставшие
достижимыми, подхватываются снова **автоматически — без рестарта** (self-heal).

---

## 🔎 Разрешение путей и бинарники

- `winws2.exe` и `winws.exe` всегда запускаются с рабочим каталогом (`current_dir`),
  установленным в `bin/`.
- Автопоиск бинарников поддерживается и в `vendor/`, и в `bin/`
  (`scripts/setup_binaries.*` и `platform/*/process.rs`).
- Движок по умолчанию **тише** (`--debug=1`), auto-hostlist держится domain-only.
- Служба ищется рядом с GUI-бинарником или в `resources/` (`find_svc_exe`).
