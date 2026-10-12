# zapret2 Desktop Supervisor & GUI

<p align="center">
  <img src="assets/logo.png" width="96" height="96" alt="zapret2 Logo" />
</p>

<p align="center">
  <strong>Высокопроизводительный, отказоустойчивый Windows-супервизор и современный GUI для zapret2.</strong><br>
  <em>High-performance, fault-tolerant Windows desktop GUI and background supervisor for zapret2.</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20x64-blue.svg" alt="Platform" />
  <img src="https://img.shields.io/badge/Backend-Rust%20%2B%20Tauri%20v2-orange.svg" alt="Backend" />
  <img src="https://img.shields.io/badge/Frontend-React%20%2B%20TypeScript%20%2B%20Tailwind-61dafb.svg" alt="Frontend" />
  <img src="https://img.shields.io/badge/Engine-winws2%20%2B%20WinDivert-green.svg" alt="Engine" />
  <img src="https://img.shields.io/badge/License-MIT-purple.svg" alt="License" />
</p>

---

## ⚡ О проекте / Overview

**zapret2 Desktop GUI** — это графический интерфейс и низкоуровневый системный супервизор для утилиты `zapret2` (`winws2.exe` / `winws.exe`) от *bol-van*, спроектированный с акцентом на **абсолютную стабильность, моментальный отклик (0ms lag) и минимальное потребление системных ресурсов (<20 МБ ОЗУ)**.

Приложение позволяет в один клик обходить замедления и блокировки **YouTube (включая 4K 60FPS)**, **Discord (голосовые каналы и стримы)** и заблокированных веб-сайтов на территории РФ без необходимости настраивать командную строку или редактировать `.bat` файлы.

---

## ✨ Ключевые возможности / Core Features

- 🛡️ **Изоляция дерева процессов через Windows Job Objects:**
  Дочерний процесс `winws2.exe` привязан к `CreateJobObjectW` с лимитом `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Даже если интерфейс будет принудительно закрыт через Диспетчер задач или упадет, ядро Windows мгновенно завершит дочерний процесс, гарантируя отсутствие «зависших» драйверов и потери интернета.
- ⚡ **Мгновенный отклик интерфейса (0ms):**
  Устранены любые задержки при старте и переключении пресетов. Кнопка и индикаторы моментально реагируют на клики.
- 🎯 **Флагманский пресет `zapret2 РФ Ultimate`:**
  Специально оптимизированная конфигурация под актуальные фильтры ТСПУ РКН:
  - **YouTube 4K 60FPS:** HTTP/3 QUIC Initial обход (UDP 443) + TCP TLS Fake с повторами и смещением `seqovl=681`.
  - **Discord Voice & STUN:** Обход голосовых шлюзов на портах `50000-50100` и `19294-19344` через бинарный payload `ACTIVE_DISCORD_UDP.bin`.
  - **Общие блокировки:** Универсальная маскировка `tls_max_ru` с выверенными оффсетами.
- 🌟 **Поддержка пресетов Flowseal v1 & zapret2 Lua v2:**
  Возможность бесшовного переключения между классическими проверенными конфигурациями Flowseal и современными модульными скриптами на языке Lua.
- 🔍 **Автономные диагностические сетевые пробы (Tokio Probes):**
  Независимый фоновый сервис в реальном времени измеряет задержку (RTT в мс), статус доступности TLS 1.3 к YouTube / Google Video CDN и официальным шлюзам Discord, выводя статус прямо в UI.
- ⚠️ **Детект и устранение конфликтов в 1 клик:**
  Автоматическое обнаружение конкурирующих процессов и служб (GoodbyeDPI, GoodCheck, сторонние инсталляции zapret, зависшие службы WinDivert) с возможностью автоматического решения конфликтов.
- 🚀 **Бесшовный автозапуск с Windows (без UAC):**
  Интеграция с Планировщиком заданий Windows (`schtasks`) с наивысшим уровнем привилегий (`RL HIGHEST`). zapret2 запускается автоматически при входе в систему с правами администратора без назойливых всплывающих окон подтверждения UAC.
- 🛠️ **Импорт собственных конфигов и готовые шаблоны:**
  Встроенный конструктор и готовые шаблоны (YouTube 4K, Discord Voice, Обход сайтов, Комбо). Парсер позволяет скопировать строку запуска прямо из любого `.bat` файла (с автоматическим распознаванием кавычек, переменных `%BIN%` и `%LISTS%`).
- 📖 **Подробное руководство по конфигам:**
  В репозиторий включено полное практическое руководство [CONFIG_GUIDE.md](CONFIG_GUIDE.md) с разбором параметров и готовыми шаблонами.
- 🌐 **Многоязычный интерфейс:**
  Полная локализация на **Русский**, **English** и **Українська** языки.
- 📊 **Высокопроизводительный потоковый журнал логов:**
  Асинхронное чтение stdout/stderr с микро-батчингом каждые 50 мс через Ring Buffer. Никаких утечек памяти и зависаний WebView.


---

## 📂 Структура репозитория / Repository Separation

### Что входит в репозиторий GitHub:
- `src/` — Исходный код React + TypeScript + Tailwind CSS UI.
- `src-tauri/` — Высокопроизводительный системный бэкенд на Rust (Tauri v2).
- `lists/` — Актуальные списки доменов и фильтров (`list-general.txt`, `list-google.txt`, `list-exclude.txt`).
- `bin/` — Lua-скрипты ядра (`zapret-antidpi.lua`, `zapret-lib.lua`) и бинарные дампы пакетов (`.bin`).
- `.github/workflows/` — Автоматический CI/CD пайплайн сборки релизов под Windows.
- `package.json`, `tsconfig.json`, `tailwind.config.js`, `Cargo.toml`.

### Что остается локально на рабочей станции (игнорируется git):
- Скомпилированные исполняемые файлы (`zapret2.exe`, `zapret2-gui.exe`).
- Системные бинарники ядра (`bin/winws2.exe`, `bin/WinDivert.dll`, `bin/WinDivert64.sys`, `bin/cygwin1.dll`) — загружаются через релизы или скрипт.
- Пользовательские конфиги (`configs/custom_profiles.json`) и временные логи.
- Тяжелые директории сборщика (`target/`, `node_modules/`, `dist/`).
- Исходные архивы распаковки вендора (`zapret-master/`, `zapret2 releases/`).

---

## 🚀 Быстрый старт / Quick Start

### Для пользователей:
1. Скачайте свежий архив **`zapret2-windows-x86_64.zip`** из вкладки [Releases](../../releases).
2. Распакуйте архив в любую удобную папку.
3. Запустите **`zapret2.exe`** от имени администратора.
4. Выберите флагманский пресет **«zapret2 РФ Ultimate»** или **«Flowseal General»** и нажмите кнопку **«Включить обход»**.

---

## 🛠️ Сборка из исходников / Building from Source

### Требования:
- **Node.js** v18+ и **npm**
- **Rust** stable (toolchain `x86_64-pc-windows-msvc` или `llvm-mingw`)
- Установленный **Tauri CLI v2**

### Пошаговая сборка:

```powershell
# 1. Клонирование репозитория
git clone https://github.com/your-username/zapret2-gui.git
cd zapret2-gui

# 2. Установка зависимостей интерфейса
npm install

# 3. Подготовка бинарных файлов ядра (если отсутствуют)
powershell -ExecutionPolicy Bypass -File scripts/setup_binaries.ps1

# 4. Сборка фронтенда
npm run build

# 5. Сборка релизного бинарника Rust
cd src-tauri
cargo build --release

# 6. Готовый исполняемый файл будет расположен в:
# src-tauri/target/release/zapret2-supervisor.exe
```

---

## 🔒 Безопасность и Windows Internals

1. **UAC Elevation:**
   Для корректной фильтрации трафика драйвер `WinDivert` регистрирует callout-драйвер в Windows Filtering Platform (WFP), что требует локальных прав администратора. В исполняемый файл встроен манифест `app.manifest` с уровнем `requireAdministrator`. При запуске без прав супервизор безопасно перезапускается с сохранением текущей рабочей директории.
2. **Безопасная очистка:**
   При выходе супервизор корректно останавливает драйвер и удаляет сервисные регистрации `WinDivert`, восстанавливая исходный сетевой стек Windows.

---

## 📄 Лицензия / License

Проект распространяется под открытой лицензией [MIT License](LICENSE).
Утилиты `winws2` и `zapret` разработаны [@bol-van](https://github.com/bol-van/zapret).
Пресеты обхода основаны на исследованиях сообщества и проекта `zapret-discord-youtube` от *Flowseal*.
