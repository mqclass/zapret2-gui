//! Pluggable camouflage-transport framework.
//!
//! # Why this exists
//! DPI desync tricks (fake/split/disorder) die on allow-listed networks where the
//! censor only lets "approved" services through or fingerprints TLS completely.
//! Camouflage transports (domain fronting, traffic mimicry, ...) are the research
//! direction for those networks — but each technique has its own config, its own
//! readiness preconditions (most importantly: **server-side exit infrastructure**),
//! and its own way of plugging into the engine launch pipeline.
//!
//! This module is the *framework*, not the breakthrough: a uniform contract
//! (`Transport` trait), a registry the UI can enumerate over IPC, strict config
//! validation with understandable errors, and fail-closed behavior — a transport
//! that is not ready **explicitly refuses** with a reason instead of silently
//! pretending to work.
//!
//! # Honest scope
//! Real circumvention of allow-lists requires exit infrastructure, mimicry
//! research and a server side that this project does not ship. The reference
//! [`DomainFrontingTransport`] below compiles and registers, but its
//! `is_available()` always returns `Unavailable` with an explicit reason.
//! See `docs/TRANSPORTS.md` for the architecture doc and the "what is missing"
//! section.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::platform::process::ProcessError;

/// Unique transport identifier used in settings, IPC and logs.
pub type TransportId = str;

/// Serializable per-transport configuration stored in [`crate::platform::settings::AppSettings`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Transport this config belongs to.
    #[serde(default)]
    pub transport_id: String,
    /// Master switch; a disabled transport behaves like "direct":
    /// transparent passthrough, skipped *before* the availability gate.
    /// "Disabled" means "off" — nothing is applied, so there is nothing
    /// to refuse, even if the transport itself is unavailable.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Transport-specific options (validated by each transport's `validate_config`).
    #[serde(default)]
    pub options: serde_json::Value,
}

fn default_true() -> bool {
    true
}

impl TransportConfig {
    /// Builds a default (empty-options) config for the given transport id.
    pub fn for_transport(transport_id: &str) -> Self {
        Self {
            transport_id: transport_id.to_string(),
            enabled: true,
            options: serde_json::Value::Object(serde_json::Map::new()),
        }
    }
}

/// Readiness verdict of a transport. A transport that cannot actually work
/// **must** return `Unavailable` with a human-readable reason — never pretend.
#[derive(Debug, Clone)]
pub enum TransportAvailability {
    Available,
    Unavailable { reason: String },
}

impl TransportAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, TransportAvailability::Available)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            TransportAvailability::Available => None,
            TransportAvailability::Unavailable { reason } => Some(reason),
        }
    }
}

/// Static metadata describing a transport (shown in the UI "Transports" section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportDescriptor {
    pub id: String,
    /// Short human-readable name (English in the backend; the frontend
    /// localizes known ids via i18n, falling back to this string).
    pub name: String,
    pub description: String,
    /// True for research-grade transports that must never be enabled silently.
    pub experimental: bool,
    /// True when the transport fundamentally needs server-side exit
    /// infrastructure (fronting endpoint, exit node, ...) to function.
    /// Any transport with this flag MUST keep `is_available()` returning
    /// `Unavailable` until the author wires real exit-infra health checks.
    pub requires_exit_infra: bool,
    /// Example JSON for `TransportConfig.options` (documentation for UI/researchers).
    pub config_example: serde_json::Value,
}

/// Serializable registry snapshot entry sent to the frontend over IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub experimental: bool,
    pub requires_exit_infra: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub default_config: TransportConfig,
    /// Example JSON for `TransportConfig.options` (from [`TransportDescriptor`];
    /// documentation for the UI / researchers).
    pub config_example: serde_json::Value,
}

/// Errors of the transport framework. All variants carry enough context to be
/// shown to the user verbatim — explicit refusal, never silence.
///
/// The message strings are Russian by design and are shown in the UI verbatim
/// (this is intentional and documented — see docs/TRANSPORTS.md, "Локализация
/// ошибок бэкенда").
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("Неизвестный транспорт «{id}». Доступные: {known}")]
    UnknownTransport { id: String, known: String },

    #[error("Транспорт «{id}» недоступен: {reason}")]
    Unavailable { id: String, reason: String },

    #[error("Некорректная конфигурация транспорта «{id}»: {message}")]
    InvalidConfig { id: String, message: String },

    #[error("Транспорт «{id}» не смог подготовить аргументы запуска: {message}")]
    ApplyFailed { id: String, message: String },
}

impl From<TransportError> for ProcessError {
    fn from(e: TransportError) -> Self {
        ProcessError::Transport(e.to_string())
    }
}

/// The pluggable-transport contract.
///
/// Implementors must keep every method honest:
/// - `is_available()` returns `Unavailable` while any precondition (most
///   notably exit infrastructure) is missing.
/// - `validate_config()` rejects anything it does not understand with a
///   message a non-expert can act on.
/// - `apply_args()` is a **pure transformation** of the final winws argument
///   list: it may only use values from an already-validated config, and must
///   never inject raw user strings (paths, domains) without allow-list
///   sanitization — the privileged engine must not load untrusted input.
pub trait Transport: Send + Sync {
    /// Stable id, e.g. `"domain-fronting"`. Used in settings and IPC.
    fn id(&self) -> &'static str;

    /// Short human-readable name.
    fn name(&self) -> &'static str;

    /// Full static metadata for UI/docs.
    fn describe(&self) -> TransportDescriptor;

    /// Readiness check: preconditions, binaries, exit infra, OS support.
    fn is_available(&self) -> TransportAvailability;

    /// Validates a user-supplied config with understandable errors.
    fn validate_config(&self, config: &TransportConfig) -> Result<(), TransportError>;

    /// Transforms the final engine argument list. Called only after the
    /// availability check and config validation passed.
    fn apply_args(
        &self,
        config: &TransportConfig,
        args: Vec<String>,
    ) -> Result<Vec<String>, TransportError>;
}

/// Registry of all known transports. Built once at startup; the UI enumerates
/// it over IPC (`list_transports`).
pub struct TransportRegistry {
    transports: Vec<Box<dyn Transport>>,
}

impl TransportRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            transports: Vec::new(),
        };
        // Built-in transports. Third-party/research transports register here.
        registry.register(Box::new(DirectTransport));
        registry.register(Box::new(DomainFrontingTransport));
        registry.register(Box::new(WarpTransport));
        registry
    }

    /// Registers a transport. Duplicate ids are ignored (first wins) so a
    /// built-in can never be shadowed by accident.
    pub fn register(&mut self, transport: Box<dyn Transport>) {
        if self.transports.iter().any(|t| t.id() == transport.id()) {
            return;
        }
        self.transports.push(transport);
    }

    pub fn get(&self, id: &TransportId) -> Option<&dyn Transport> {
        self.transports.iter().find(|t| t.id() == id).map(|t| t.as_ref())
    }

    pub fn ids(&self) -> Vec<String> {
        self.transports.iter().map(|t| t.id().to_string()).collect()
    }

    /// Serializable snapshot for the UI, including live availability.
    pub fn list(&self) -> Vec<TransportView> {
        self.transports
            .iter()
            .map(|t| {
                let desc = t.describe();
                let availability = t.is_available();
                TransportView {
                    id: desc.id.clone(),
                    name: desc.name,
                    description: desc.description,
                    experimental: desc.experimental,
                    requires_exit_infra: desc.requires_exit_infra,
                    available: availability.is_available(),
                    unavailable_reason: availability.reason().map(|s| s.to_string()),
                    default_config: TransportConfig::for_transport(&desc.id),
                    config_example: desc.config_example.clone(),
                }
            })
            .collect()
    }

    /// Full pipeline for one launch: lookup → disabled check (passthrough) →
    /// availability gate → config validation → argument transformation.
    /// Every refusal is explicit.
    pub fn apply(
        &self,
        id: &TransportId,
        config: Option<&TransportConfig>,
        args: Vec<String>,
    ) -> Result<Vec<String>, TransportError> {
        let transport = self.get(id).ok_or_else(|| TransportError::UnknownTransport {
            id: id.to_string(),
            known: self.ids().join(", "),
        })?;

        // 1. Master switch FIRST: a disabled transport is a transparent
        //    passthrough, same as "direct". Nothing is applied, so the
        //    availability gate is deliberately skipped: "disabled" means
        //    "off", and there is nothing to refuse. (A stale selection of an
        //    unavailable transport is still surfaced in the UI via
        //    list()/TransportView, and set_active_transport refuses to store
        //    unavailable transports in the first place.)
        let effective = config.cloned().unwrap_or_else(|| TransportConfig::for_transport(id));
        if !effective.enabled {
            return Ok(args);
        }

        // 2. Availability gate: a transport that cannot work must refuse
        //    before we even look at its config.
        if let TransportAvailability::Unavailable { reason } = transport.is_available() {
            return Err(TransportError::Unavailable {
                id: id.to_string(),
                reason,
            });
        }

        // 3. Validate the effective config (stored one, or defaults).
        transport.validate_config(&effective)?;

        // 4. Transform the launch arguments.
        transport.apply_args(&effective, args)
    }
}

impl Default for TransportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_REGISTRY: OnceLock<TransportRegistry> = OnceLock::new();

/// Process-wide transport registry (built once, read-only afterwards).
pub fn global_registry() -> &'static TransportRegistry {
    GLOBAL_REGISTRY.get_or_init(TransportRegistry::new)
}

// ---------------------------------------------------------------------------
// Built-in transports
// ---------------------------------------------------------------------------

/// The "no transport" transport: current behavior, always available.
/// Selecting it (or nothing) keeps the engine launch exactly as before.
pub struct DirectTransport;

impl Transport for DirectTransport {
    fn id(&self) -> &'static str {
        "direct"
    }

    fn name(&self) -> &'static str {
        "Direct (no transport)"
    }

    fn describe(&self) -> TransportDescriptor {
        TransportDescriptor {
            id: self.id().to_string(),
            name: self.name().to_string(),
            description: "Standard winws2 launch without camouflage transports. \
                This is the current, well-tested behavior."
                .to_string(),
            experimental: false,
            requires_exit_infra: false,
            config_example: serde_json::json!({}),
        }
    }

    fn is_available(&self) -> TransportAvailability {
        TransportAvailability::Available
    }

    fn validate_config(&self, _config: &TransportConfig) -> Result<(), TransportError> {
        Ok(())
    }

    fn apply_args(
        &self,
        _config: &TransportConfig,
        args: Vec<String>,
    ) -> Result<Vec<String>, TransportError> {
        Ok(args)
    }
}

/// Typed options for the domain-fronting reference transport.
#[derive(Debug, Deserialize)]
struct DomainFrontingOptions {
    /// Public CDN/hosting domain whose TLS certificate fronts the connection
    /// (the SNI the censor sees).
    front_domain: String,
    /// The real destination SNI, sent inside the encrypted channel
    /// (requires a cooperating fronting endpoint — NOT shipped).
    target_sni: String,
    /// Handshake timeout in milliseconds.
    #[serde(default = "default_fronting_timeout_ms")]
    timeout_ms: u64,
}

fn default_fronting_timeout_ms() -> u64 {
    8000
}

/// Very small domain allow-list check: letters, digits, dots, hyphens;
/// at least one dot; no spaces or shell metacharacters. This is *syntax*
/// validation only — it says nothing about the domain being suitable.
fn is_plausible_domain(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.contains('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !s.starts_with('.')
        && !s.starts_with('-')
        && !s.ends_with('.')
        && !s.ends_with('-')
        && !s.contains("..")
}

/// Reference skeleton of a domain-fronting camouflage transport.
///
/// HONEST STATUS: this is a *structural* reference — trait implementation,
/// config schema, validation — with **no working data path**. Domain fronting
/// needs a cooperating fronting endpoint and exit infrastructure on the server
/// side, which this project does not deploy. `is_available()` therefore always
/// returns `Unavailable` with an explicit reason, and `apply_args()` refuses
/// as a second line of defense.
///
/// TODO (for a researcher picking this up):
/// 1. Deploy or point at a real fronting-capable endpoint (CDN / proxy with
///    a valid cert for `front_domain` that forwards by inner Host/SNI).
/// 2. Implement exit-infra health checks (reachability, cert validation) and
///    make `is_available()` depend on them instead of the constant below.
/// 3. Decide the actual data path: winws2 desync args cannot do fronting by
///    themselves — this likely needs a local helper process (e.g. a TLS
///    forwarder) spawned *before* winws2, with `--wf-*` filters adjusted.
/// 4. Re-run `validate_config()` against the real endpoint's constraints.
pub struct DomainFrontingTransport;

impl Transport for DomainFrontingTransport {
    fn id(&self) -> &'static str {
        "domain-fronting"
    }

    fn name(&self) -> &'static str {
        "Domain Fronting (skeleton)"
    }

    fn describe(&self) -> TransportDescriptor {
        TransportDescriptor {
            id: self.id().to_string(),
            name: self.name().to_string(),
            description: "Reference skeleton: makes TLS handshakes present the SNI \
                of an allowed front domain while the real destination travels \
                inside the encrypted channel. REQUIRES server-side exit \
                infrastructure which is NOT shipped — currently unavailable by design."
                .to_string(),
            experimental: true,
            requires_exit_infra: true,
            config_example: serde_json::json!({
                "front_domain": "allowed-cdn.example.com",
                "target_sni": "real-destination.example.net",
                "timeout_ms": 8000
            }),
        }
    }

    fn is_available(&self) -> TransportAvailability {
        // TODO: replace with real exit-infra health checks (see struct docs).
        // Until a fronting endpoint + exit node exist, claiming availability
        // would give users a false sense of protection. Refuse explicitly.
        TransportAvailability::Unavailable {
            reason: "Нет серверной части: domain fronting требует fronting-endpoint \
                и exit-инфраструктуру, которые не поставляются с zapret2-gui. \
                Транспорт зарегистрирован как каркас для исследований. \
                Подробнее: docs/TRANSPORTS.md, раздел «Чего не хватает»."
                .to_string(),
        }
    }

    fn validate_config(&self, config: &TransportConfig) -> Result<(), TransportError> {
        let opts: DomainFrontingOptions = serde_json::from_value(config.options.clone())
            .map_err(|e| TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: format!(
                    "Конфиг должен быть JSON-объектом с полями front_domain, target_sni \
                     (строки) и timeout_ms (число). Ошибка разбора: {}",
                    e
                ),
            })?;

        if !is_plausible_domain(&opts.front_domain) {
            return Err(TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: format!(
                    "«front_domain» = «{}» не похож на домен (латиница, цифры, точки, дефисы, \
                     минимум одна точка). Пример: allowed-cdn.example.com",
                    opts.front_domain
                ),
            });
        }
        if !is_plausible_domain(&opts.target_sni) {
            return Err(TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: format!(
                    "«target_sni» = «{}» не похож на домен (латиница, цифры, точки, дефисы, \
                     минимум одна точка).",
                    opts.target_sni
                ),
            });
        }
        if opts.front_domain.eq_ignore_ascii_case(&opts.target_sni) {
            return Err(TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: "«front_domain» и «target_sni» совпадают — маскировка бессмысленна: \
                    front-домен должен отличаться от реального назначения."
                    .to_string(),
            });
        }
        if !(100..=30_000).contains(&opts.timeout_ms) {
            return Err(TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: format!(
                    "«timeout_ms» = {} вне допустимого диапазона 100–30000 мс.",
                    opts.timeout_ms
                ),
            });
        }
        Ok(())
    }

    fn apply_args(
        &self,
        _config: &TransportConfig,
        _args: Vec<String>,
    ) -> Result<Vec<String>, TransportError> {
        // Second line of defense: even if the availability gate were bypassed,
        // the skeleton must never pretend to transform the launch.
        // TODO: implement the real data path (see struct docs) before removing this.
        Err(TransportError::ApplyFailed {
            id: self.id().to_string(),
            message: "Каркас транспорта: путь данных не реализован, серверная часть отсутствует. \
                См. docs/TRANSPORTS.md."
                .to_string(),
        })
    }
}

/// WARP transport via Cloudflare (sing-box MASQUE sidecar).
///
/// # Architecture (built, stage 4)
///
/// The original plan - boringtun plus a hand-rolled WireGuard client -
/// was dropped in favour of the pragmatic path (design section 1):
/// a vendored sing-box fork (shtorm-7/sing-box-extended, pinned 2.7.3)
/// runs as a sidecar under the supervisor, dials WARP over MASQUE
/// (HTTP/2 `CONNECT-UDP` to a hardcoded endpoint), and exposes a TUN
/// interface through wintun. Registration is done by the engine
/// itself and cached on disk.
///
/// This transport is the *contract* for that mode, not the lifecycle:
/// `is_available` checks the binaries, `validate_config` rejects
/// unknown options, and `apply_args` prepares the desync launch
/// (tunnel exclusion filter + registration recipe). The sidecar's
/// lifecycle belongs to `SingBoxManager` (stage 3) and the supervisor
/// (stage 4), driven by the `warp_enabled` setting - not by the
/// transport selection. Selecting "warp" here only prepares the
/// desync launch; the tunnel itself starts when the switch is on.
pub struct WarpTransport;

/// v1 has no user-facing options: the endpoint is hardcoded in the
/// engine, the MTU and scope are fixed by the config generator.
/// `deny_unknown_fields` keeps the old boringtun-era options
/// (`endpoint`, `mtu`, `full_tunnel`) from silently passing - a
/// stored config from the skeleton era must fail loudly, not be
/// misread as meaningful.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WarpOptions {}

/// Availability check over already-resolved binaries, split out so
/// it is unit-testable without a real installation next to the test
/// binary. The full `sing-box check` runs only at start time inside
/// the manager (stage 3) - deliberately NOT here: `is_available` is
/// called on every UI enumeration, and a config check needs a
/// generated config, which does not exist yet at enumeration time.
fn check_warp_binaries(
    binaries: &crate::platform::process::ZapretBinaries,
) -> TransportAvailability {
    let Some(binary) = binaries.sing_box_exe.as_deref() else {
        return TransportAvailability::Unavailable {
            reason: "Движок sing-box не найден в установке: WARP-режим недоступен. \
                Обновите приложение - движок поставляется вместе с ним."
                .to_string(),
        };
    };
    // wintun is a Windows driver library: on Linux sing-box uses the
    // kernel TUN device, so the dll is not a precondition there (the
    // Linux ZapretBinaries always reports None).
    #[cfg(windows)]
    if binaries.wintun_dll.is_none() {
        return TransportAvailability::Unavailable {
            reason: "wintun.dll не найден в установке: без него нет TUN-интерфейса, \
                а значит, и туннеля. Обновите приложение."
                .to_string(),
        };
    }
    match crate::engine::singbox::singbox_version(binary) {
        Ok(_version) => TransportAvailability::Available,
        Err(e) => TransportAvailability::Unavailable {
            reason: format!("sing-box не запускается: {e}"),
        },
    }
}

impl Transport for WarpTransport {
    fn id(&self) -> &'static str {
        "warp"
    }

    fn name(&self) -> &'static str {
        "Cloudflare WARP"
    }

    fn describe(&self) -> TransportDescriptor {
        TransportDescriptor {
            id: self.id().to_string(),
            name: self.name().to_string(),
            description: "Туннель Cloudflare WARP (sing-box, MASQUE): часть трафика \
                идёт через сеть Cloudflare в обход DPI. Запасной выход, а не \
                замена десинку: включается отдельным выключателем."
                .to_string(),
            experimental: true,
            // The exit is Cloudflare's public network, not self-hosted
            // infrastructure - nothing for the author to run or monitor.
            // (The framework's "must stay Unavailable" rule for this flag
            // therefore does not apply: the precondition is the vendored
            // binary, checked by is_available.)
            requires_exit_infra: false,
            config_example: serde_json::json!({}),
        }
    }

    fn is_available(&self) -> TransportAvailability {
        match crate::platform::process::resolve_binaries() {
            Ok(binaries) => check_warp_binaries(&binaries),
            Err(e) => TransportAvailability::Unavailable {
                reason: format!("Не удалось найти установку движка: {e}"),
            },
        }
    }

    fn validate_config(&self, config: &TransportConfig) -> Result<(), TransportError> {
        let _opts: WarpOptions = serde_json::from_value(config.options.clone()).map_err(|e| {
            TransportError::InvalidConfig {
                id: self.id().to_string(),
                message: format!(
                    "У WARP-режима в текущей версии нет настроек: конфиг должен быть \
                     пустым объектом {{}}. Ошибка разбора: {}",
                    e
                ),
            }
        })?;
        Ok(())
    }

    fn apply_args(
        &self,
        _config: &TransportConfig,
        args: Vec<String>,
    ) -> Result<Vec<String>, TransportError> {
        // The transport framework does not know the engine, so it
        // detects it best-effort from the arguments (fail-closed to
        // "not winws2"). The supervisor's own wiring applies the same
        // transformation with certainty and is idempotent with this
        // one - a double application changes nothing.
        //
        // The tunnel-switch gate is NOT here: apply_args is a pure
        // transformation of a validated config (see the trait docs),
        // and the gate lives in apply_active_transport, which forces
        // the warp config disabled when warp_enabled is off - so
        // this method is never reached for a launch without the
        // tunnel. Callers must go through apply_active_transport;
        // calling apply_args directly bypasses the switch.
        let mut args = args;
        if crate::engine::tunnel_capture::args_look_like_winws2(&args) {
            crate::engine::tunnel_capture::apply_warp_args(&mut args, true);
        }
        Ok(args)
    }
}

/// Validates an arbitrary JSON string against a transport's config schema.
///
/// Used by the IPC `validate_transport_config` command so the UI can show
/// field-level errors *before* the user tries to start the engine.
pub fn validate_transport_config_json(
    transport_id: &str,
    config_json: &str,
) -> Result<(), TransportError> {
    let registry = global_registry();
    let transport = registry.get(transport_id).ok_or_else(|| TransportError::UnknownTransport {
        id: transport_id.to_string(),
        known: registry.ids().join(", "),
    })?;
    let options: serde_json::Value =
        serde_json::from_str(config_json).map_err(|e| TransportError::InvalidConfig {
            id: transport_id.to_string(),
            message: format!("Некорректный JSON: {}", e),
        })?;
    let config = TransportConfig {
        transport_id: transport_id.to_string(),
        enabled: true,
        options,
    };
    transport.validate_config(&config)
}

/// Effective launch config for the active transport: the WARP
/// transport additionally requires the tunnel switch (`warp_enabled`,
/// design section 6) - its launch args exist only to serve the
/// tunnel, so with the switch off the transport is forced disabled
/// even when selected in the UI. Pure, so it can be unit-tested
/// without touching settings on disk; `apply_active_transport`
/// feeds it the loaded values.
fn effective_launch_config(
    transport_id: &str,
    config: Option<TransportConfig>,
    warp_enabled: bool,
) -> Option<TransportConfig> {
    if transport_id == "warp" && !warp_enabled {
        let mut disabled = config.unwrap_or_else(|| TransportConfig::for_transport("warp"));
        disabled.enabled = false;
        Some(disabled)
    } else {
        config
    }
}

/// Applies the active transport (from settings) to the final argument list.
///
/// `None`/unknown handling is fail-closed and explicit; "direct" (or unset) is
/// a transparent passthrough that keeps the legacy launch path untouched.
pub fn apply_active_transport(
    args: Vec<String>,
) -> Result<(Vec<String>, Option<String>), TransportError> {
    let settings = crate::platform::settings::load_settings();
    let transport_id = settings
        .active_transport_id
        .clone()
        .unwrap_or_else(|| "direct".to_string());
    // Stage 4 (review m3): the WARP launch args (the WinDivert
    // exclusion filter + the registration recipe) must not be applied
    // with the tunnel switch off. The gate lives here, not in
    // WarpTransport::apply_args: the trait contract says apply_args
    // is a pure transformation of an already-validated config, and
    // this function is its only production caller (it already loads
    // settings). Forcing `enabled` off reuses the registry's
    // disabled-passthrough and keeps the "applied" report honest (a
    // disabled transport is not reported as applied).
    let config = effective_launch_config(
        &transport_id,
        settings.transport_configs.get(&transport_id).cloned(),
        settings.warp_enabled,
    );
    let final_args = global_registry().apply(&transport_id, config.as_ref(), args)?;
    // A disabled transport is a transparent passthrough (see apply()), so it
    // is *not* reported as applied — the log must not claim otherwise.
    let enabled = config.as_ref().map(|c| c.enabled).unwrap_or(true);
    let applied = if transport_id == "direct" || !enabled {
        None
    } else {
        Some(transport_id)
    };
    Ok((final_args, applied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_builtin_transports() {
        let registry = TransportRegistry::new();
        let ids = registry.ids();
        assert!(ids.contains(&"direct".to_string()));
        assert!(ids.contains(&"domain-fronting".to_string()));
    }

    #[test]
    fn direct_transport_is_passthrough_and_available() {
        let registry = TransportRegistry::new();
        let args = vec!["--wf-tcp=80,443".to_string()];
        let out = registry.apply("direct", None, args.clone()).unwrap();
        assert_eq!(out, args);
        let t = registry.get("direct").unwrap();
        assert!(t.is_available().is_available());
    }

    #[test]
    fn unknown_transport_fails_closed_with_known_list() {
        let registry = TransportRegistry::new();
        let err = registry.apply("nope", None, vec![]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nope"), "message names the bad id: {}", msg);
        assert!(msg.contains("direct"), "message lists known ids: {}", msg);
    }

    #[test]
    fn domain_fronting_refuses_with_explicit_reason() {
        let registry = TransportRegistry::new();
        let t = registry.get("domain-fronting").unwrap();
        match t.is_available() {
            TransportAvailability::Unavailable { reason } => {
                assert!(reason.contains("серверной части"), "reason is explicit: {}", reason);
            }
            TransportAvailability::Available => panic!("skeleton must not claim availability"),
        }
        let desc = t.describe();
        assert!(desc.experimental);
        assert!(desc.requires_exit_infra);

        // The apply pipeline refuses at the availability gate.
        let err = registry.apply("domain-fronting", None, vec![]).unwrap_err();
        assert!(matches!(err, TransportError::Unavailable { .. }));
    }

    #[test]
    fn domain_fronting_config_validation_is_strict_but_clear() {
        let registry = TransportRegistry::new();
        let t = registry.get("domain-fronting").unwrap();
        let cfg = |options: serde_json::Value| TransportConfig {
            transport_id: "domain-fronting".to_string(),
            enabled: true,
            options,
        };

        // Valid shape passes validation (availability is a separate gate).
        t.validate_config(&cfg(serde_json::json!({
            "front_domain": "allowed-cdn.example.com",
            "target_sni": "real.example.net",
            "timeout_ms": 8000
        })))
        .unwrap();

        // Bad domain is rejected with a message naming the field.
        let err = t
            .validate_config(&cfg(serde_json::json!({
                "front_domain": "not a domain!!",
                "target_sni": "real.example.net"
            })))
            .unwrap_err();
        assert!(err.to_string().contains("front_domain"));

        // front == target is pointless camouflage.
        let err = t
            .validate_config(&cfg(serde_json::json!({
                "front_domain": "same.example.com",
                "target_sni": "same.example.com"
            })))
            .unwrap_err();
        assert!(err.to_string().contains("бессмысленна"));

        // Timeout out of range.
        let err = t
            .validate_config(&cfg(serde_json::json!({
                "front_domain": "a.example.com",
                "target_sni": "b.example.net",
                "timeout_ms": 999999
            })))
            .unwrap_err();
        assert!(err.to_string().contains("timeout_ms"));

        // Non-object JSON gets a structural error, not a panic.
        let err = t
            .validate_config(&cfg(serde_json::json!("just a string")))
            .unwrap_err();
        assert!(matches!(err, TransportError::InvalidConfig { .. }));
    }

    #[test]
    fn disabled_transport_is_transparent_passthrough() {
        let registry = TransportRegistry::new();
        // A disabled transport behaves like "direct": transparent passthrough,
        // even when the transport itself is unavailable. The availability gate
        // is skipped on purpose — "disabled" means "off": nothing is applied,
        // so there is nothing to refuse. (An *enabled* but unavailable
        // transport still refuses explicitly — see
        // domain_fronting_refuses_with_explicit_reason.)
        let cfg = TransportConfig {
            transport_id: "domain-fronting".to_string(),
            enabled: false,
            options: serde_json::Value::Null,
        };
        let args = vec!["--wf-tcp=80,443".to_string()];
        let out = registry
            .apply("domain-fronting", Some(&cfg), args.clone())
            .unwrap();
        assert_eq!(out, args);
    }

    #[test]
    fn warp_descriptor_is_experimental_without_exit_infra() {
        let registry = TransportRegistry::new();
        let t = registry.get("warp").unwrap();
        assert_eq!(t.id(), "warp");
        assert_eq!(t.name(), "Cloudflare WARP");
        let desc = t.describe();
        assert!(desc.experimental);
        // The exit is Cloudflare's public network, not self-hosted infra.
        assert!(!desc.requires_exit_infra);
        assert_eq!(desc.config_example, serde_json::json!({}));
    }

    #[test]
    fn warp_config_validation_rejects_unknown_options() {
        let registry = TransportRegistry::new();
        let t = registry.get("warp").unwrap();
        let cfg = |options: serde_json::Value| TransportConfig {
            transport_id: "warp".to_string(),
            enabled: true,
            options,
        };

        // v1 has no options: the empty object passes.
        t.validate_config(&cfg(serde_json::json!({}))).unwrap();

        // The old boringtun-era options must fail loudly, not be
        // silently swallowed (a stored skeleton-era config).
        let err = t
            .validate_config(&cfg(serde_json::json!({
                "endpoint": "162.159.192.1:2408",
                "mtu": 1280,
                "full_tunnel": false
            })))
            .unwrap_err();
        assert!(matches!(err, TransportError::InvalidConfig { .. }));
        assert!(err.to_string().contains("пустым объектом"));

        // Non-object JSON gets a structural error, not a panic.
        let err = t
            .validate_config(&cfg(serde_json::json!("just a string")))
            .unwrap_err();
        assert!(matches!(err, TransportError::InvalidConfig { .. }));
    }

    fn test_binaries(
        sing_box: Option<std::path::PathBuf>,
        wintun: Option<std::path::PathBuf>,
    ) -> crate::platform::process::ZapretBinaries {
        crate::platform::process::ZapretBinaries {
            bin_dir: std::path::PathBuf::from("/nonexistent"),
            winws2_exe: std::path::PathBuf::from("/nonexistent/winws2.exe"),
            winws_exe: None,
            winws2_split_exe: None,
            sing_box_exe: sing_box,
            wintun_dll: wintun,
            windivert_dll: std::path::PathBuf::from("/nonexistent/WinDivert.dll"),
            windivert_sys: std::path::PathBuf::from("/nonexistent/WinDivert64.sys"),
            lists_dir: None,
        }
    }

    #[test]
    fn warp_availability_needs_the_sidecar_binary() {
        // No sing-box at all.
        let binaries = test_binaries(None, None);
        match check_warp_binaries(&binaries) {
            TransportAvailability::Unavailable { reason } => {
                assert!(reason.contains("sing-box"), "reason: {reason}");
            }
            TransportAvailability::Available => panic!("must not claim availability"),
        }

        // A binary path that does not execute.
        let binaries = test_binaries(
            Some(std::path::PathBuf::from("/nonexistent/sing-box")),
            Some(std::path::PathBuf::from("/nonexistent/wintun.dll")),
        );
        match check_warp_binaries(&binaries) {
            TransportAvailability::Unavailable { reason } => {
                assert!(reason.contains("не запускается"), "reason: {reason}");
            }
            TransportAvailability::Available => panic!("must not claim availability"),
        }

        // A binary that exits non-zero on `version` (/bin/false).
        #[cfg(unix)]
        {
            let binaries = test_binaries(
                Some(std::path::PathBuf::from("/bin/false")),
                Some(std::path::PathBuf::from("/nonexistent/wintun.dll")),
            );
            assert!(!check_warp_binaries(&binaries).is_available());
        }
    }

    /// A fake sing-box that prints a version line: the Available
    /// branch, deterministically, without the real binary. Unix-only:
    /// a shell script with a shebang is the cheapest executable.
    #[cfg(unix)]
    #[test]
    fn warp_availability_accepts_a_working_binary() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("zapret2-warp-avail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("fake-sing-box");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho 'sing-box version 1.14.3-extended-2.7.3'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let binaries = test_binaries(
            Some(fake),
            Some(std::path::PathBuf::from("/nonexistent/wintun.dll")),
        );
        assert!(check_warp_binaries(&binaries).is_available());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn warp_apply_args_prepares_winws2_and_skips_winws1() {
        let registry = TransportRegistry::new();
        let t = registry.get("warp").unwrap();
        let cfg = TransportConfig::for_transport("warp");

        // winws2: filter + recipe.
        let args = vec![
            "--wf-tcp-out=80,443".to_string(),
            "--new=general".to_string(),
            "--filter-tcp=443".to_string(),
        ];
        let out = t.apply_args(&cfg, args).unwrap();
        assert!(out[0].starts_with("--wf-raw-filter=("), "args: {out:?}");
        assert!(out.iter().any(|a| a == "--new=warp_api"), "args: {out:?}");

        // winws v1 (Flowseal): untouched - no filter, no recipe.
        let args = vec!["--dpi-desync=fake".to_string()];
        let out = t.apply_args(&cfg, args.clone()).unwrap();
        assert_eq!(out, args);

        // Ambiguous (neither marker): fails closed, untouched.
        let args = vec!["--wf-tcp=80,443".to_string()];
        let out = t.apply_args(&cfg, args.clone()).unwrap();
        assert_eq!(out, args);
    }

    #[test]
    fn warp_launch_config_requires_tunnel_switch() {
        // Switch off: warp is forced disabled, whatever the stored
        // config says - so the registry's disabled-passthrough keeps
        // the filter and the recipe out of the launch (review m3).
        let enabled = TransportConfig::for_transport("warp");
        assert!(enabled.enabled);
        let out = effective_launch_config("warp", Some(enabled), false).unwrap();
        assert!(!out.enabled);
        // No stored config at all: still disabled, not a
        // default-enabled config.
        let out = effective_launch_config("warp", None, false).unwrap();
        assert!(!out.enabled);
        // Switch on: the stored config passes through untouched.
        let enabled = TransportConfig::for_transport("warp");
        let out = effective_launch_config("warp", Some(enabled.clone()), true).unwrap();
        assert_eq!(out.enabled, enabled.enabled);
        assert_eq!(out.transport_id, enabled.transport_id);
        // Other transports are unaffected by the tunnel switch.
        let enabled = TransportConfig::for_transport("domain-fronting");
        let out = effective_launch_config("domain-fronting", Some(enabled.clone()), false).unwrap();
        assert_eq!(out.enabled, enabled.enabled);
    }

    #[test]
    fn duplicate_registration_does_not_shadow_builtin() {
        let mut registry = TransportRegistry::new();
        registry.register(Box::new(DirectTransport));
        assert_eq!(registry.ids().iter().filter(|i| *i == "direct").count(), 1);
    }

    #[test]
    fn transport_config_serde_roundtrip() {
        let cfg = TransportConfig::for_transport("domain-fronting");
        let json = serde_json::to_string(&cfg).unwrap();
        let back: TransportConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.transport_id, "domain-fronting");
        assert!(back.enabled);
        // Missing fields default instead of failing (forward compatibility).
        let minimal: TransportConfig = serde_json::from_str("{}").unwrap();
        assert!(minimal.enabled);
    }
}
