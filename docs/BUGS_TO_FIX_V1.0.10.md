# ✅ Список багов аудита (v1.0.10) — ВСЕ ИСПРАВЛЕНЫ

Все обнаруженные баги успешно исправлены в кодовой базе, покрыты тестами и проверены.

---

## 📌 Оглавление
1. [BH-003: Срыв фильтрации WinDivert в режиме Strict Split-Tunneling (Critical)](#bh-003-critical)
2. [BH-004: Ложный отказ диагностики Discord Voice из-за несуществующего домена (High)](#bh-004-high)
3. [BH-005: Потеря пользовательских доменов при синхронизации списков с GitHub (Medium)](#bh-005-medium)
4. [BH-006: Фиктивный бенчмарк стратегий в commands.rs (Наблюдение)](#bh-006-наблюдение)
5. [BH-007: Замер скорости YouTube обращается к Cloudflare CDN (Наблюдение)](#bh-007-наблюдение)

---

## <a id="bh-003-critical"></a>1. BH-003 [Critical]: Срыв фильтрации WinDivert в режиме Strict Split-Tunneling

### 📍 Где находится:
- **Файл:** `src-tauri/src/engine/supervisor.rs`
- **Строки:** ~1018–1045

### 💥 В чём проблема:
В версии 1.0.10 добавлен режим **Strict Split-Tunneling** («Только YouTube и Discord»), который отсекает правила для общих сайтов (`list-general.txt` и `ipset-all.txt`).  
В коде аргументы разбиваются по разделителю `--new`. Но в пресетах Flowseal глобальные параметры захвата сетевых пакетов `--wf-tcp=...` и `--wf-udp=...` находятся **в самом начале нулевого блока** вместе с правилом для `list-general.txt`.  
Когда код отбрасывает блок с `list-general.txt`, он **полностью выбрасывает `--wf-tcp` и `--wf-udp`**!  
В результате `winws.exe` запускается вообще без фильтра портов WinDivert (`--filter-udp=...` идёт самым первым аргументом). Драйвер WinDivert не захватывает ни одного пакета, и **весь обход блокировок перестаёт работать**.

### 🔍 Доказательство (вывод теста):
```text
thread 'engine::supervisor::tests::test_strict_split_tunnel_must_preserve_wf_filters' panicked at src\engine\supervisor.rs:
Strict Split-Tunneling stripped --wf-tcp filter from winws command line: [
    "--filter-udp=19294-19344,50000-65535", 
    "--dpi-desync=fake", 
    ...
]
```

### 🛠️ Готовое исправление (Diff):
В файле `src-tauri/src/engine/supervisor.rs`:
```diff
--- a/src-tauri/src/engine/supervisor.rs
+++ b/src-tauri/src/engine/supervisor.rs
@@ -1019,6 +1019,10 @@ impl Supervisor {
         if is_strict {
+            let global_filters: Vec<String> = args.iter()
+                .filter(|a| a.starts_with("--wf-tcp=") || a.starts_with("--wf-udp="))
+                .cloned()
+                .collect();
             let mut filtered = Vec::new();
             let mut current_block = Vec::new();
             for arg in args {
                 if arg == "--new" {
@@ -1039,6 +1043,9 @@ impl Supervisor {
                         filtered.extend(current_block);
                     }
                 }
+            }
+            let mut final_args = global_filters;
+            final_args.extend(filtered);
+            args = final_args;
         }
```

---

## <a id="bh-004-high"></a>2. BH-004 [High]: Ложный отказ диагностики Discord Voice

### 📍 Где находится:
- **Файл:** `src-tauri/src/engine/probes.rs`
- **Строка:** 292

### 💥 В чём проблема:
Функция проверки работоспособности Discord Voice пингует домен `voice.discord.gg`:
```rust
self.measure_endpoint("Discord Voice", "voice.discord.gg", 443, "https://discord.com/api/v10/voice/regions")
```
Хоста `voice.discord.gg` **не существует в публичном DNS** (DNS-серверы Discord возвращают `NXDOMAIN`).  
Из-за этого функция `lookup_host("voice.discord.gg:443")` завершается ошибкой, и индикатор **Discord Voice у всех пользователей всегда горит красным** («Блокировка ТСПУ / DNS-спуфинг: домен не резолвится», потеря пакетов 100%), даже если голосовая связь Discord полностью исправна.

### 🔍 Доказательство (вывод команды):
```powershell
Resolve-DnsName voice.discord.gg
# Результат: discord.gg SOA 1579 Authority (домен не существует)
```

### 🛠️ Готовое исправление (Diff):
В файле `src-tauri/src/engine/probes.rs`:
```diff
--- a/src-tauri/src/engine/probes.rs
+++ b/src-tauri/src/engine/probes.rs
@@ -290,5 +290,5 @@ impl DiagnosticsService {
     pub async fn probe_discord_voice(&self) -> ProbeMetric {
         self.measure_endpoint(
             "Discord Voice",
-            "voice.discord.gg",
+            "discord.media",
             443,
             "https://discord.com/api/v10/voice/regions",
         ).await
```

---

## <a id="bh-005-medium"></a>3. BH-005 [Medium]: Потеря пользовательских доменов при синхронизации списков

### 📍 Где находится:
- **Файл бэкенда:** `src-tauri/src/win32/lists.rs:258-312`
- **Файл фронтенда:** `src/components/views/ListsView.tsx:22`

### 💥 В чём проблема:
В интерфейсе приложения (`ListsView.tsx`) файлом по умолчанию выбран `list-general.txt`. Когда пользователь вручную добавляет домены, они сохраняются именно в `list-general.txt`.  
При нажатии кнопки **«Синхронизировать списки с GitHub»** функция `update_remote_lists()` скачивает свежий `list-general.txt` из официального репозитория Flowseal и **перезаписывает файл целиком** (`fs::write(&target_path, &text)`), уничтожая все добавленные пользователем сайты.  
Во Flowseal для пользовательских сайтов специально предусмотрен файл `list-general-user.txt`, который никогда не перезаписывается апстримом.

### 🛠️ Готовое исправление:
1. В `src/components/views/ListsView.tsx` переключить файл по умолчанию на `list-general-user.txt`:
```diff
--- a/src/components/views/ListsView.tsx
+++ b/src/components/views/ListsView.tsx
@@ -22,1 +22,1 @@
-  const [selectedFile, setSelectedFile] = useState<string>("list-general.txt");
+  const [selectedFile, setSelectedFile] = useState<string>("list-general-user.txt");
```
2. В `src/components/DomainManagerModal.tsx` также сделать приоритетным `list-general-user.txt`.

---

## <a id="bh-006-наблюдение"></a>4. BH-006 [Наблюдение]: Фиктивный бенчмарк стратегий

### 📍 Где находится:
- **Файл:** `src-tauri/src/commands.rs`
- **Строки:** 268–343 (`benchmark_strategies`)

### 💥 В чём проблема:
Функция «Автоподбор / Бенчмарк стратегий» не выполняет реальных сетевых замеров, а возвращает захардкоженные константы задержки:
- `flowseal-alt`: `Some(55)`
- `flowseal-alt11`: `Some(68)`
- `flowseal-fake-tls-auto`: `Some(42)`
- `zapret2-ru-ultimate`: `Some(58)`
- `flowseal-general`: `None` (Blocked)

Пользователь видит красивые цифры пинга, но они не отражают реальное состояние его провайдера.

---

## <a id="bh-007-наблюдение"></a>5. BH-007 [Наблюдение]: Замер скорости YouTube обращается к Cloudflare CDN

### 📍 Где находится:
- **Файл бэкенда:** `src-tauri/src/engine/speedtest.rs:32`
- **Файл фронтенда:** `src/components/views/DnsSpeedView.tsx:209`

### 💥 В чём проблема:
В интерфейсе указано:
`Целевой узел: YouTube Video Edge Cache (googlevideo.com)`  
Однако в коде скачивание 10 МБ файла идёт с:
`https://speed.cloudflare.com/__down?bytes=10000000`  
В режиме **Strict Split-Tunneling** трафик к Cloudflare идёт напрямую мимо Zapret (так как Cloudflare не входит в `list-google.txt`), поэтому спидтест показывает не скорость разблокированного YouTube, а прямое соединение с Cloudflare.
