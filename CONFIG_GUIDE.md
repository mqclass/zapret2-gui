# 📖 Руководство по созданию и настройке конфигов zapret2 & winws

Это подробное руководство объясняет, как устроены конфигурации обхода DPI (ТСПУ РКН), что означает каждый параметр командной строки `winws` / `winws2`, и как легко создавать собственные эффективные профили в **zapret2 GUI**.

---

## ⚡ Оглавление
1. [Как работает обход ТСПУ](#1-как-работает-обход-тспу)
2. [Разбор параметров winws](#2-разбор-параметров-winws)
3. [Разделитель `--new` (мульти-профили)](#3-разделитель---new-мульти-профили)
4. [Готовые проверенные шаблоны](#4-готовые-проверенные-шаблоны)
   - [YouTube 4K 60FPS](#шаблон-1-youtube-4k-60fps)
   - [Discord Voice & STUN](#шаблон-2-discord-voice--stun)
   - [Блокировки сайтов (РКН)](#шаблон-3-блокировки-сайтов-ркн)
   - [Универсальный комбо-профиль](#шаблон-4-универсальный-комбо-профиль)
5. [Как перенести конфиг из любого .bat файла](#5-как-перенести-конфиг-из-любого-bat-файла)

---

## 1. Как работает обход ТСПУ

Технические средства противодействия угрозам (ТСПУ) анализируют сетевые пакеты по сигнатурам (DPI — Deep Packet Inspection). В частности:
- **HTTPS сайты и YouTube**: фильтр анализирует первый пакет TLS-рукопожатия — `ClientHello`, в котором открытым текстом передаётся имя запрашиваемого домена (SNI, Server Name Indication).
- **YouTube 4K (HTTP/3 QUIC)**: работает по протоколу UDP на 443 порту. ТСПУ глушит или замедляет пакеты `QUIC Initial`.
- **Discord Voice**: использует UDP-трафик на портах `50000-50100` и `19294-19344`.

### Главные методы обхода:
1. **`fake` (Фальшивый пакет)**: перед реальным пакетом отправляется поддельный пакет с невалидным таймштампом (`ts`) или контрольной суммой (`badsum`). Фильтр ТСПУ видит фейк и «запоминает» его, а конечный сервер отбрасывает фейк и принимает ваши настоящие данные.
2. **`multisplit` / `split2` (Разрезание пакета)**: реальный пакет разрезается на несколько кусочков так, чтобы доменное имя в SNI оказалось разорванным между двумя пакетами. Фильтр не может распознать домен в одном кусочке и пропускает соединение.
3. **`seqovl` (Sequence Overlap)**: смещение номеров последовательности TCP. Номер последовательности фейка сдвигается на точное количество байт (`seqovl=681` для YouTube, `seqovl=664` для сайтов), из-за чего сигнатура блокировки в ТСПУ ломается на 100%.

---

## 2. Разбор параметров winws

| Параметр | Назначение | Пример использования |
| :--- | :--- | :--- |
| `--wf-tcp=...` | Порты, которые перехватывает драйвер WinDivert для TCP | `--wf-tcp=80,443,2053,2083,8443` |
| `--wf-udp=...` | Порты, которые перехватывает WinDivert для UDP | `--wf-udp=443,19294-19344,50000-50100` |
| `--filter-tcp=...` | Порты для конкретного правила внутри профиля | `--filter-tcp=443` |
| `--filter-udp=...` | Порты UDP для конкретного правила | `--filter-udp=50000-50100` |
| `--hostlist=...` | Путь к файлу со списком доменов | `--hostlist="lists/list-general.txt"` |
| `--hostlist-exclude=...` | Список доменов-исключений (не фильтровать) | `--hostlist-exclude="lists/list-exclude.txt"` |
| `--dpi-desync=...` | Методы десинхронизации (через запятую) | `--dpi-desync=fake,multisplit` |
| `--dpi-desync-split-pos=...` | Позиция разрезания пакета (1 = после 1 байта) | `--dpi-desync-split-pos=1` |
| `--dpi-desync-split-seqovl=...` | Смещение перекрытия TCP | `--dpi-desync-split-seqovl=681` |
| `--dpi-desync-fooling=...` | Способ маскировки фейка (`ts`, `badsum`, `badseq`) | `--dpi-desync-fooling=ts` |
| `--dpi-desync-repeats=...` | Количество повторов фейкового пакета | `--dpi-desync-repeats=8` |
| `--dpi-desync-fake-tls=...` | Бинарный файл поддельного TLS ClientHello | `--dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin"` |
| `--dpi-desync-fake-quic=...` | Бинарный файл поддельного QUIC Initial | `--dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin"` |
| `--dpi-desync-fake-discord=...` | Бинарный файл поддельного UDP Discord | `--dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin"` |
| `--new` | **Разделитель правил** (начинает новый блок фильтрации) | `--new` |

---

## 3. Разделитель `--new` (мульти-профили)

Параметр `--new` позволяет в одной команде настроить разный обход для разных сервисов!

Например:
```
[ПРАВИЛО 1: YouTube QUIC]
--filter-udp=443 --dpi-desync=fake --dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin"
--new
[ПРАВИЛО 2: Discord Голос]
--filter-udp=50000-50100 --dpi-desync=fake --dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin"
--new
[ПРАВИЛО 3: YouTube 4K TCP]
--filter-tcp=443 --hostlist="lists/list-google.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681
--new
[ПРАВИЛО 4: Сайты РКН]
--filter-tcp=80,443 --hostlist="lists/list-general.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=664
```

---

## 4. Готовые проверенные шаблоны

### Шаблон 1: YouTube 4K 60FPS
Идеально для стабильного воспроизведения YouTube без буферизации на 1080p, 2K и 4K:
```
--wf-tcp=80,443 --wf-udp=443 --filter-udp=443 --hostlist="lists/list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin" --new --filter-tcp=443 --hostlist="lists/list-google.txt" --ip-id=zero --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin"
```

### Шаблон 2: Discord Voice & STUN
Восстанавливает подключение к голосовым комнатам, веб-камерам и демонстрации экрана:
```
--wf-tcp=2053,2083,2087,2096,8443 --wf-udp=19294-19344,50000-50100 --filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-fake-stun="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-repeats=6 --new --filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin"
```

### Шаблон 3: Блокировки сайтов (РКН)
Открывает заблокированные трекеры, библиотеки, форумы и ресурсы:
```
--wf-tcp=80,443 --filter-tcp=80,443 --hostlist="lists/list-general.txt" --hostlist-exclude="lists/list-exclude.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=664 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-tls="bin/stun2.bin" --dpi-desync-fake-tls="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-http="bin/tls_clienthello_max_ru.bin"
```

### Шаблон 4: Универсальный комбо-профиль
Комплексный профиль «всё в одном» для дома:
```
--wf-tcp=80,443,2053,2083,2087,2096,8443 --wf-udp=443,19294-19344,50000-50100 --filter-udp=443 --hostlist="lists/list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin" --new --filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-fake-stun="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-repeats=6 --new --filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin" --new --filter-tcp=443 --hostlist="lists/list-google.txt" --ip-id=zero --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin" --new --filter-tcp=80,443 --hostlist="lists/list-general.txt" --hostlist-exclude="lists/list-exclude.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=664 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-tls="bin/stun2.bin" --dpi-desync-fake-tls="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-http="bin/tls_clienthello_max_ru.bin"
```

---

## 5. Как перенести конфиг из любого .bat файла

Если вы нашли рабочий `.bat` файл (например, из репозиториев Flowseal, GoodCheck или форумов):
1. Откройте `.bat` файл в Блокноте.
2. Найдите строку, начинающуюся с `start ... winws.exe ...`.
3. Скопируйте всё, что идёт после `winws.exe` (или даже всю строку целиком со `start`).
4. В **zapret2 GUI** нажмите **«+ Свой конфиг»**.
5. Вставьте скопированный текст в поле **«Параметры запуска»**.
6. Встроенный парсер zapret2 GUI **автоматически**:
   - Удалит префиксы `start`, `call`, `/min` и `winws.exe`.
   - Заменит переменные `%BIN%` и `%LISTS%` на актуальные пути.
   - Корректно обработает кавычки и пробелы.
7. Нажмите кнопку **«Сохранить конфиг»** — ваш профиль готов к запуску в 1 клик!
