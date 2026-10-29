# zapret2 Desktop Supervisor & GUI — Архитектура проекта

Настоящий документ описывает структурированную организацию каталогов и архитектуру проекта **zapret2 Desktop Supervisor & GUI**.

---

## 📁 Структура каталогов

```text
zapret2/
├── bin/                          # Авторитетные бинарные файлы и полезные нагрузки
│   ├── winws2.exe                # Официальный движок DPI bypass (bol-van zapret2 v2)
│   ├── winws.exe                 # Движок первого поколения (Flowseal zapret v1)
│   ├── WinDivert.dll             # Пользовательская библиотека перехвата пакетов WFP
│   ├── WinDivert64.sys           # Драйвер ядра WinDivert x64 WFP
│   ├── cygwin1.dll               # POSIX среда выполнения для winws/winws2
│   ├── *.bin                     # Бинарные дампы ClientHello, QUIC Initial, STUN
│   └── windows-x86_64/           # Дополнительные утилиты zapret2 (ip2net, mdig, killall)
│
├── configs/                      # Пользовательские настройки, кастомные профили, состояние
│   └── zapret2_config.json       # Активная конфигурация приложения
│
├── docs/                         # Техническая документация, отчёты и архитектура
│   ├── ARCHITECTURE.md           # Этот документ
│   └── BUG_HUNTER_REPORT.md      # Результаты независимого аудита безопасности и качества
│
├── lists/                        # Списки доменов, IP и исключений
│   ├── list-general.txt          # Общий список заблокированных доменов
│   ├── list-google.txt           # Домены Google / YouTube
│   ├── ipset-all.txt             # IP-диапазоны сервисов
│   ├── list-exclude-user.txt     # Пользовательские исключения по доменам
│   └── ipset-exclude-user.txt    # Пользовательские исключения по IP-адресам
│
├── logs/                         # Журналы работы супервизора и движка
│   ├── zapret2.log               # Активный журнал событий
│   └── user_feedback_logs.txt    # Диагностические логи пользователей
│
├── scripts/                      # Скрипты сборки, обновления и развертывания
│   ├── setup_binaries.ps1        # Автоматическая загрузка/валидация upstream-бинарников
│   ├── update_zapret2.bat        # Быстрое обновление локального zapret2.exe после сборки
│   └── push_to_github.bat        # Вспомогательный скрипт пуша в репозиторий GitHub
│
├── src/                          # Frontend (React 19, TypeScript, Tailwind CSS, Vite)
│   ├── components/               # UI компоненты (Dashboard, Strategies, Logs, etc.)
│   ├── i18n/                     # Локализация интерфейса (ru, en, uk)
│   ├── App.tsx                   # Корневой компонент приложения
│   └── main.tsx                  # Точка входа React
│
├── src-tauri/                    # Backend Supervisor (Rust, Tauri v2)
│   ├── src/
│   │   ├── commands/             # Команды Tauri IPC (supervisor, autostart, dns, etc.)
│   │   ├── engine/               # Супервизор процессов, логгер, DNS-резолвер
│   │   └── platform/windows/   # Интеграция с Win32 API (Job Objects, UAC, WinDivert)
│   ├── capabilities/             # Права доступа Tauri v2 (window dragging, shell, etc.)
│   ├── icons/                    # Иконки приложения
│   ├── app.manifest              # UAC-манифест уровня Administrator (RT_MANIFEST)
│   ├── Cargo.toml                # Зависимости Rust
│   └── tauri.conf.json           # Конфигурация Tauri окна и сборки
│
├── strategies/                   # Lua-стратегии zapret2 (antidpi, auto, obfs)
│   ├── zapret-antidpi.lua        # Универсальный официальный десинк zapret2
│   └── zapret-auto.lua           # Автоматический подбор параметров десинка
│
├── vendor/                       # Upstream репозитории и исходники (игнорируются в git)
│   ├── zapret-discord-youtube-main/  # Flowseal/zapret-discord-youtube
│   ├── Zapret-GUI-main/              # Референсный GUI
│   ├── zapret2 releases/             # Распакованные релизы bol-van/zapret2
│   └── zapret2-master/               # Полные исходники C/Makefile ядра zapret2
│
├── Запустить zapret2.bat         # Пользовательский ярлык быстрого запуска
├── Запустить zapret2.vbs         # Безоконный запуск через VBS
├── zapret2.exe                   # Готовый бинарник супервизора (Windows x64)
├── CONFIG.md                     # Инструкции и правила выполнения задач
├── CONFIG_GUIDE.md               # Руководство по настройке стратегий и параметров
├── README.md                     # Описание проекта на русском
├── README_EN.md                  # Описание проекта на английском
└── LICENSE                       # Лицензия MIT
```

---

## ⚙️ Принципы работы и разрешения путей

1. **Единый бинарник супервизора:**
   - Единственным исполняемым файлом приложения является `zapret2.exe` в корне.
   - Двойное дублирование (`zapret2-gui.exe`) устранено.

2. **Изоляция стороннего кода (`vendor/`):**
   - Все распакованные исходники и релизы сторонних проектов изолированы в `vendor/`.
   - `scripts/setup_binaries.ps1` и Rust-супервизор в `src-tauri/src/platform/windows/process.rs` поддерживают автопоиск как в `vendor/`, так и в `bin/`.

3. **Разрешение путей во время работы:**
   - `winws2.exe` и `winws.exe` всегда запускаются с рабочим каталогом (`current_dir`), установленным в `bin/`.
   - Все дочерние процессы привязаны к Windows Job Object с флагом `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, исключая зависание драйвера WinDivert или зомби-процессов при закрытии GUI.
