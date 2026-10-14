# zapret2 Desktop Supervisor & GUI

<p align="center">
  <img src="assets/logo.png" width="96" height="96" alt="zapret2 Logo" />
</p>

<p align="center">
  <strong>Высокопроизводительный, отказоустойчивый Windows-супервизор и современный GUI для zapret2.</strong><br>
  <em>High-performance, fault-tolerant Windows desktop GUI and background supervisor for zapret2.</em>
</p>

<p align="center">
  <strong><a href="README.md">🇷🇺 Русский</a></strong> | <strong><a href="README_EN.md">🇬🇧 English</a></strong>
</p>

<p align="center">
  <a href="https://github.com/mqclass/zapret2-gui/releases"><img src="https://img.shields.io/github/v/release/mqclass/zapret2-gui?color=brightgreen&label=Latest%20Release" alt="Latest Release" /></a>
  <a href="https://github.com/mqclass/zapret2-gui/releases"><img src="https://img.shields.io/github/downloads/mqclass/zapret2-gui/total?color=blue&label=Downloads" alt="Downloads" /></a>
  <a href="https://github.com/mqclass/zapret2-gui/stargazers"><img src="https://img.shields.io/github/stars/mqclass/zapret2-gui?style=flat&color=yellow" alt="Stars" /></a>
  <img src="https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011%20x64-blue.svg" alt="Platform" />
  <img src="https://img.shields.io/badge/Backend-Rust%20%2B%20Tauri%20v2-orange.svg" alt="Backend" />
  <img src="https://img.shields.io/badge/Frontend-React%20%2B%20TypeScript%20%2B%20Tailwind-61dafb.svg" alt="Frontend" />
  <img src="https://img.shields.io/badge/License-MIT-purple.svg" alt="License" />
</p>

---

> [!CAUTION]
> ### ⚠️ ОСТОРОЖНО: ФЕЙКИ И МОШЕННИКИ
>
> - **Я не веду никакие другие страницы, группы в Telegram или каналы на YouTube.**
> - Программа распространяется **ИСКЛЮЧИТЕЛЬНО на официальном [GitHub](https://github.com/mqclass/zapret2-gui)**.
> - У меня **нет** YouTube-каналов, я **не** заливаю zapret2-gui на форумы, файлообменники или сторонние сайты.
> - **Не скачивайте zapret2-gui с других ресурсов** — это могут быть вирусные и модифицированные вредоносные сборки.
> - Если вы наткнулись на что-то вне этой страницы GitHub, что распространяется от моего лица — **это ФЕЙК**.

---

## ⚡ О проекте / Overview

**zapret2 Desktop GUI** — это графический интерфейс и низкоуровневый системный супервизор для утилиты `zapret2` (`winws2.exe` / `winws.exe`) от *bol-van*, спроектированный с акцентом на **абсолютную стабильность, моментальный отклик (0ms lag) и минимальное потребление системных ресурсов (<20 МБ ОЗУ)**.

Приложение позволяет в один клик обходить замедления и блокировки **YouTube (включая 4K 60FPS)**, **Discord (голосовые каналы и стримы)** и заблокированных веб-сайтов на территории РФ без необходимости настраивать командную строку или редактировать `.bat` файлы.

---

## ✨ Ключевые возможности / Core Features

- 🛡️ **Изоляция дерева процессов через Windows Job Objects:**
  Дочерний процесс `winws2.exe` привязан к `CreateJobObjectW` с лимитом `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Даже если интерфейс будет принудительно закрыт через Диспетчер задач или упадет, ядро Windows мгновенно завершит дочерний процесс, гарантируя отсутствие «зависших» драйверов и потери интернета.
- 📌 **Сворачивание в трей при закрытии (System Tray):**
  При нажатии на крестик окно сворачивается в системный трей (область уведомлений Windows рядом с часами), а фоновый обход продолжает работать без прерываний. Левый клик разворачивает окно, а правый вызывает контекстное меню управления и полного выхода.
- 🎮 **Раздельное туннелирование & Game Mode (Split-Tunneling):**
  - **Full Bypass:** Полный обход YouTube, Discord и заблокированных сайтов/ipsets.
  - **Strict Split-Tunnel:** Перехват ИСКЛЮЧИТЕЛЬНО трафика YouTube и Discord. Игры, банки, Госуслуги и торренты идут 100% напрямую без участия драйвера.
  - **Game Mode:** Активирует Flowseal Game Filter на портах UDP 1024-65535 (`cutoff=n3`) для разблокировки лобби и голосовых чатов (Apex Legends, Steam, Roblox) с нулевым добавленным пингом (0ms lag).
- 🩺 **1-Click Авто-Доктор (Восстановление сети):**
  Комплексная автоматическая диагностика: завершение зависших процессов, сброс службы перехвата WinDivert, очистка DNS-кэша (`ipconfig /flushdns`), сброс ARP-таблицы, активация TCP Timestamps и проверка шлюза.
- 🛡️ **Менеджер DNS & Защита от подмены (Anti-DNS-Spoofing):**
  Управление DNS на активном сетевом адаптере в 1 клик: Cloudflare (1.1.1.1), Comss.one (анти-блокировка для РФ), Google (8.8.8.8), Quad9 (9.9.9.9) и системный DHCP.
- ⚡ **Реальный тест скорости YouTube 4K Stream:**
  Прямое измерение пропускной способности видеопотока googlevideo в Мбит/с через текущую стратегию обхода с вердиктом качества (4K 60FPS, 1080p, 720p).
- 🔄 **OTA Синхронизация списков доменов и IP:**
  Загрузка самых свежих списков (`list-general.txt`, `list-google.txt`, `ipset-all.txt`) напрямую из репозитория Flowseal в один клик с резервным копированием `.bak`.
- 🌐 **Менеджер сайтов и доменов (Hostlist Manager):**
  Удобное визуальное окно для добавления своих доменов в обход (`list-general.txt`, `list-general-user.txt`, `list-exclude.txt`), быстрого поиска сайтов, их удаления и проверки открытия в браузере.
- 🌟 **Поддержка пресетов Flowseal v1 & zapret2 Lua v2:**
  Возможность бесшовного переключения между классическими проверенными конфигурациями Flowseal и современными модульными скриптами на языке Lua.
- 🔍 **Автономные диагностические сетевые пробы (Tokio Probes):**
  Независимый фоновый сервис в реальном времени измеряет задержку (RTT в мс), статус доступности TLS 1.3 к YouTube / Google Video CDN и официальным шлюзам Discord, выводя статус прямо в UI.
- ⚠️ **Детект и устранение конфликтов в 1 клик:**
  Автоматическое обнаружение конкурирующих процессов и служб (GoodbyeDPI, GoodCheck, сторонние инсталляции zapret, зависшие службы WinDivert) с возможностью автоматического решения конфликтов.
- 🚀 **Бесшовный автозапуск с Windows (без UAC):**
  Интеграция с Планировщиком заданий Windows (`schtasks`) с наивысшим уровнем привилегий (`RL HIGHEST`). zapret2 запускается автоматически при входе в систему в свернутом виде (`--minimized`) без назойливых всплывающих окон подтверждения UAC.
- 🛠️ **Импорт собственных конфигов и готовые шаблоны:**
  Встроенный конструктор и готовые шаблоны (YouTube 4K, Discord Voice, Обход сайтов, Комбо). Парсер позволяет скопировать строку запуска прямо из любого `.bat` файла (с автоматическим распознаванием кавычек, переменных `%BIN%` и `%LISTS%`).
- 📖 **Подробное руководство по конфигам:**
  В репозиторий включено полное практическое руководство [CONFIG_GUIDE.md](CONFIG_GUIDE.md) с разбором параметров и готовыми шаблонами.
- 🌍 **Многоязычный интерфейс:**
  Полная локализация на **Русский**, **English** и **Українська** языки.
- 📊 **Высокопроизводительный потоковый журнал логов:**
  Асинхронное чтение stdout/stderr с микро-батчингом каждые 50 мс через Ring Buffer. Никаких утечек памяти и зависаний WebView.

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
git clone https://github.com/mqclass/zapret2-gui.git
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

# 6. Готовый исполняемый файл:
# src-tauri/target/release/zapret2-supervisor.exe
```

---

## 🔒 Безопасность и Windows Internals

1. **UAC Elevation:**
   Для корректной фильтрации трафика драйвер `WinDivert` регистрирует callout-драйвер в Windows Filtering Platform (WFP), что требует локальных прав администратора. В исполняемый файл встроен манифест `app.manifest` с уровнем `requireAdministrator`. При запуске без прав супервизор безопасно перезапускается с сохранением текущей рабочей директории.
2. **Безопасная очистка:**
   При выходе супервизор корректно останавливает драйвер и удаляет сервисные регистрации `WinDivert`, восстанавливая исходный сетевой стек Windows.

---

## 🤝 Благодарности и оригинальные проекты / Credits

Выражаем огромную благодарность авторам оригинальных инструментов и исследований, благодаря которым стал возможен этот проект:

- 🛠️ [**bol-van/zapret2**](https://github.com/bol-van/zapret2) — оригинальное ядро zapret / zapret2 (`winws2.exe`, `winws.exe`), WinDivert фильтрация и алгоритмы десинхронизации TCP/UDP.
- ⚡ [**Flowseal/zapret-discord-youtube**](https://github.com/Flowseal/zapret-discord-youtube) — наборы проверенных стратегий, списков доменов и конфигураций десинхронизации для YouTube и Discord.
- 🎨 [**medvedeff-true/Zapret-Gui**](https://github.com/medvedeff-true/Zapret-Gui) — вдохновение графического интерфейса, UX-решений и идеи менеджера сайтов.

---

## 📄 Лицензия / License
 
 Проект распространяется под открытой лицензией [MIT License](LICENSE).
 
 ---
 
 ## 🏷️ Теги и ключевые слова / Topics & Tags
 
 `zapret` · `zapret2` · `winws` · `winws2` · `WinDivert` · `dpi` · `dpi-bypass` · `youtube-unblock` · `discord-fix` · `bypass-censorship` · `anti-censorship` · `goodbyedpi` · `russia-dpi` · `tauri` · `rust` · `react` · `typescript` · `windows-11` · `windows-10` · `gui`

