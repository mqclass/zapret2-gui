# 📋 Список багов аудита Bug-Hunter (v1.0.11) — ДЛЯ ИСПРАВЛЕНИЯ

> **Статус аудита:** **FAIL** (найдено 1 High, 2 Medium, 1 Observation).  
> **Важно:** В соответствии с указанием пользователя, **исправления в исходный код не вносились**. Ниже представлены детальные описания, доказательства тестов и готовые diff-патчи для самостоятельного применения.

---

## 📌 Оглавление
1. [BH-008 [High]: Правила Windows NRPT Suffix игнорируют Apex-домены (discord.com, youtube.com, x.com)](#bh-008-high)
2. [BH-009 [Medium]: Деструктивный сброс DNS адаптера при активации Split-DNS](#bh-009-medium)
3. [BH-010 [Medium]: Загрязнение list-google.txt доменами Discord GIF и Tenor/Giphy](#bh-010-medium)
4. [BH-011 [Observation]: Синтетический расчет задержек в benchmark_strategies](#bh-011-observation)

---

## <a id="bh-008-high"></a>1. BH-008 [High]: Правила Windows NRPT Suffix игнорируют Apex-домены

### 📍 Где находится:
- **Файл:** [`src-tauri/src/win32/dns.rs`](file:///c:/Users/winstone/Desktop/хз%20зачем/zapret2/src-tauri/src/win32/dns.rs#L157-L186)
- **Строки:** 157–186 (`SPLIT_DNS_NAMESPACES`)

### 💥 В чём проблема:
В массиве `SPLIT_DNS_NAMESPACES` все доменные пространства начинаются с точки:
```rust
pub const SPLIT_DNS_NAMESPACES: &[&str] = &[
    ".discord.gg",
    ".discord.com",
    ".youtube.com",
    ".x.com",
    ".t.me",
    ".rutracker.org",
    ...
];
```
Согласно спецификации Windows DNS Client и командлета `Add-DnsClientNrptRule`:
- Пространство имён, начинающееся с точки (`.domain.com`), является **Suffix Rule** (правилом суффикса).
- Правило суффикса перехватывает **только поддомены** (FQDN, заканчивающиеся на `.domain.com`, например `www.youtube.com`, `gateway.discord.gg`).
- При обращении к корневому домену (**Apex domain**) `discord.com`, `discord.gg`, `youtube.com`, `x.com`, `t.me`, `rutracker.org`:
  Строка `'discord.com'` **НЕ заканчивается на `'.discord.com'`**!

В результате:
При активном режиме **Выборочного Split-DNS** все запросы к корневым сайтам (`https://discord.com`, `https://x.com`, `https://t.me`, `https://rutracker.org`, `https://youtube.com`) **проходят мимо NRPT** и отправляются на системный DNS сетевого адаптера (к провайдеру с цензурой РКН). Пользователь получает `DNS_PROBE_FINISHED_NXDOMAIN` или заглушку провайдера!

### 🔍 Доказательство (вывод теста):
```text
thread 'win32::dns::tests::test_split_dns_must_match_apex_domains' panicked at src\win32\dns.rs:
Apex domain 'discord.com' was not matched by any NRPT rule in SPLIT_DNS_NAMESPACES! It will bypass Split-DNS and query ISP DNS.
```

### 🛠️ Готовое исправление (Diff):
В функции `apply_split_dns_nrpt` генерировать оба варианта — и FQDN корневого домена, и суффикс `.*`:
```diff
--- a/src-tauri/src/win32/dns.rs
+++ b/src-tauri/src/win32/dns.rs
@@ -226,7 +226,17 @@ pub fn apply_split_dns_nrpt(primary: &str, secondary: &str) -> Result<(), Strin
     let ns_list = SPLIT_DNS_NAMESPACES
         .iter()
-        .map(|ns| format!("'{}'", ns))
+        .flat_map(|ns| {
+            let clean = ns.trim_start_matches('.');
+            vec![
+                format!("'.{}'", clean),
+                format!("'{}'", clean),
+            ]
+        })
         .collect::<Vec<_>>()
         .join(",");
```

---

## <a id="bh-009-medium"></a>2. BH-009 [Medium]: Деструктивный сброс DNS адаптера при активации Split-DNS

### 📍 Где находится:
- **Файл:** [`src-tauri/src/win32/dns.rs`](file:///c:/Users/winstone/Desktop/хз%20зачем/zapret2/src-tauri/src/win32/dns.rs#L330-L342)
- **Строки:** 330–342

### 💥 В чём проблема:
При переключении в режим `DnsScope::Split` выполняется PowerShell-команда:
```powershell
Set-DnsClientServerAddress -InterfaceAlias '$adapter' -ResetServerAddresses -ErrorAction SilentlyContinue
```
Таблица NRPT в Windows работает как **политика поверх существующего сетевого стека**. Она автономно направляет запросы выбранных доменов в `NameServers` правила, а все остальные запросы оставляет текущему DNS адаптера.  
Вызов `-ResetServerAddresses` безусловно затирает любые статические настройки DNS (например, корпоративный Active Directory DNS `10.0.0.1`, домашний Pi-hole/AdGuard Home `192.168.1.50` или локальный шлюз) и сбрасывает сетевой адаптер на DHCP провайдера. На ПК со статическими IP сеть за пределами обойденных сайтов полностью падает.

### 🛠️ Готовое исправление (Diff):
```diff
--- a/src-tauri/src/win32/dns.rs
+++ b/src-tauri/src/win32/dns.rs
@@ -330,14 +330,7 @@ pub fn set_system_dns(provider_id: &str, scope: Option<&str>) -> Result<DnsStat
         match effective_scope {
             crate::win32::settings::DnsScope::Split => {
-                // In Split mode: ensure adapter uses DHCP for everything else, and route bypass domains via NRPT
-                let reset_script = format!(
-                    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; Set-DnsClientServerAddress -InterfaceAlias '{}' -ResetServerAddresses -ErrorAction SilentlyContinue",
-                    adapter
-                );
-                let _ = Command::new("powershell")
-                    .creation_flags(CREATE_NO_WINDOW)
-                    .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &reset_script])
-                    .output();
+                // In Split mode: NRPT cleanly overlays on top of the adapter without destroying user's static/custom DNS

                 apply_split_dns_nrpt(&target.primary, &target.secondary)?;
             }
```

---

## <a id="bh-010-medium"></a>3. BH-010 [Medium]: Загрязнение list-google.txt доменами Discord GIF и Tenor/Giphy

### 📍 Где находится:
- **Файл:** [`src-tauri/src/win32/lists.rs`](file:///c:/Users/winstone/Desktop/хз%20зачем/zapret2/src-tauri/src/win32/lists.rs#L324-L334)
- **Строки:** 324–334 (`update_remote_lists`)

### 💥 В чём проблема:
При синхронизации списков с GitHub выполняется условие:
```rust
if (filename == "list-general.txt" || filename == "list-google.txt") && seen.insert(domain.to_string()) {
    merged_lines.push(domain.to_string());
}
```
Где в массив `essentials` включены: `tenor.com`, `media.tenor.com`, `giphy.com`, `dl.google.com` и др.  
Из-за общего условия `|| filename == "list-google.txt"` все домены поиска гифок Discord (`tenor.com`, `giphy.com`) прописываются в `list-google.txt`.  
В ядре `winws` для `list-google.txt` применяются специальные профили для YouTube (многократные повторы фейков `repeats=6`, смещения для видеосерверов Google). Запросы к сторонним GIF CDN получают не предназначенные для них параметры десинхронизации.

### 🛠️ Готовое исправление (Diff):
```diff
--- a/src-tauri/src/win32/lists.rs
+++ b/src-tauri/src/win32/lists.rs
@@ -324,14 +324,20 @@ pub async fn update_remote_lists() -> Result<ListSyncReport, String> {
-                            let essentials = [
-                                "tenor.com", "media.tenor.com", "c.tenor.com", "t.tenor.com", "api.tenor.com",
-                                "tenor.co", "gifdeliverynetwork.com", "giphy.com", "media.giphy.com",
-                                "api.giphy.com", "i.giphy.com", "dl.google.com",
-                            ];
-                            for domain in essentials {
-                                if (filename == "list-general.txt" || filename == "list-google.txt") && seen.insert(domain.to_string()) {
-                                    merged_lines.push(domain.to_string());
-                                }
-                            }
+                            if filename == "list-google.txt" {
+                                if seen.insert("dl.google.com".to_string()) {
+                                    merged_lines.push("dl.google.com".to_string());
+                                }
+                            } else if filename == "list-general.txt" {
+                                let discord_gif_essentials = [
+                                    "tenor.com", "media.tenor.com", "c.tenor.com", "t.tenor.com", "api.tenor.com",
+                                    "tenor.co", "gifdeliverynetwork.com", "giphy.com", "media.giphy.com",
+                                    "api.giphy.com", "i.giphy.com",
+                                ];
+                                for domain in discord_gif_essentials {
+                                    if seen.insert(domain.to_string()) {
+                                        merged_lines.push(domain.to_string());
+                                    }
+                                }
+                            }
```

---

## <a id="bh-011-observation"></a>4. BH-011 [Observation]: Синтетический расчет задержек в benchmark_strategies

### 📍 Где находится:
- **Файл:** [`src-tauri/src/commands.rs`](file:///c:/Users/winstone/Desktop/хз%20зачем/zapret2/src-tauri/src/commands.rs#L301-L346)
- **Строки:** 301–346

### 💥 В чём суть:
В функции `benchmark_strategies()` вычисляется общий `base_ping` текущего соединения, а задержки для пресетов рассчитываются синтетически:
- `flowseal-alt11`: `base_ping + 6`
- `flowseal-fake-tls-auto`: `base_ping - 4`
- `zapret2-ru-ultimate`: `base_ping + 2`
- `flowseal-general`: жестко зашит `Blocked`

Это обусловлено архитектурой (переключение стратегии на лету требует перезапуска `winws` и прерывания активных сессий пользователя). Рекомендуется отображать в UI плашку «Оценка на основе сетевого профиля», чтобы не вводить пользователя в заблуждение, будто каждая стратегия была запущена изолированно в отдельном процессе.
