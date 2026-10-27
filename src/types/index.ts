export type SupervisorState =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "error";

export type ProbeStatus = "operational" | "degraded" | "blocked" | "checking";

export interface ProbeMetric {
  target: string;
  status: ProbeStatus;
  latency_ms: number | null;
  dns_ms?: number | null;
  tcp_ms?: number | null;
  http_status?: number | null;
  packet_loss_pct: number;
  last_checked_epoch: number;
  ip_address?: string | null;
  message: string;
}

export interface DiagnosticsReport {
  youtube: ProbeMetric;
  discord: ProbeMetric;
  discord_voice?: ProbeMetric;
  telegram?: ProbeMetric;
  twitter?: ProbeMetric;
  cloudflare?: ProbeMetric;
  timestamp: number;
}

export interface StrategyProbeResult {
  strategy_id: string;
  strategy_name: string;
  youtube_status: ProbeStatus;
  youtube_ms: number | null;
  discord_status: ProbeStatus;
  discord_ms: number | null;
  voice_status: ProbeStatus;
  voice_ms: number | null;
  score: number;
  average_ping_ms: number | null;
  engine_running: boolean;
  error: string | null;
}

export interface SupervisorStatus {
  state: SupervisorState;
  pid: number | null;
  active_strategy: string | null;
  /** Active camouflage transport id applied at launch, if any. */
  active_transport: string | null;
  binary_path: string | null;
  uptime_secs: number;
  total_logs_buffered: number;
  /** Last fatal engine error code, e.g. "windivert_conflict"; null otherwise. */
  last_engine_error: string | null;
}

export interface TgProxyStatus {
  state: SupervisorState;
  host: string;
  port: number;
  /** 32-hex-char MTProto secret; empty when not generated yet. */
  secret: string;
  /** Ready-to-use tg://proxy?... deep link; empty when no secret yet. */
  tg_link: string;
  uptime_secs: number;
  last_error: string | null;
  /** Connection mode: "auto" | "cfproxy" | "direct". */
  mode: string;
  /** Whether LAN access is enabled (bridge bound to 0.0.0.0). */
  lan_enabled: boolean;
  /** This machine's LAN IPv4 for the phone QR; null when LAN is off. */
  lan_ip: string | null;
  /** Stale-secret pattern live: >=5 bad handshakes in 60 s. */
  stale_secret_detected: boolean;
  /** This bridge start replaced a stored-but-unusable secret. */
  secret_rotated: boolean;
  /** Bad handshakes inside the current 60 s window. */
  bad_handshake_count: number;
  /** Monotonic rotation generation for this process. */
  rotation_count: number;
  /** Monotonic stale-episode generation; survives bridge restarts
      within the process, never reset by a good handshake. */
  stale_trip_count?: number;
}

export interface ZapretBinaries {
  bin_dir: string;
  winws2_exe: string;
  windivert_dll: string;
  windivert_sys: string;
}

export interface LuaStrategy {
  id: string;
  name: string;
  filename: string;
  path: string;
  size_bytes: number;
  description: string;
  engine: "winws" | "winws2";
  category: "flowseal" | "zapret2" | "custom";
  custom_args?: string[] | null;
}

export interface LogEntry {
  timestamp: number;
  level: "stdout" | "stderr" | "info" | "warn" | "error" | "tgproxy";
  message: string;
  /** Client-side monotonic id for stable React keys (assigned at ingest, not from backend). */
  id?: number;
}

export interface ConflictItem {
  name: string;
  kind: "process" | "service";
  pid: number | null;
  description: string;
}

export interface SystemStatusResponse {
  is_elevated: boolean;
  supervisor: SupervisorStatus;
  diagnostics: DiagnosticsReport;
  binaries: ZapretBinaries | null;
  conflicts: ConflictItem[];
  autostart_enabled: boolean;
}

/** Lightweight heartbeat response: supervisor state + elevation only. */
export interface SupervisorStatusResponse {
  is_elevated: boolean;
  supervisor: SupervisorStatus;
}

export type SplitTunnelMode = "full" | "strict" | "game_mode";

export interface AppSettings {
  split_tunnel_mode: SplitTunnelMode;
  game_filter_tcp: boolean;
  game_filter_udp: boolean;
  game_ports_tcp: string;
  game_ports_udp: string;
  dns_provider: string;
  last_strategy_id?: string | null;
  auto_start_bypass?: boolean;
  auto_doctor_on_start?: boolean;
  notifications_enabled?: boolean;
  auto_update_enabled?: boolean;
  last_update_check?: number | null;
  /** Watchdog: background strategy re-checks with auto-escalation. */
  watchdog_enabled?: boolean;
  /** Watchdog check interval in minutes. */
  watchdog_interval_minutes?: number;
  /** UI accent color override ("#RRGGBB"); null/undefined = built-in violet. */
  accent_color?: string | null;
  /** Active camouflage transport id (`null`/`undefined` = direct launch). */
  active_transport_id?: string | null;}

/** Release information returned by the `check_for_updates` IPC command. */
export interface UpdateInfo {
  current_version: string;
  latest_version: string;
  update_available: boolean;
  release_notes: string;
  release_url: string;
  asset_name: string;
  asset_size: number;
  published_at: string;
}

/** Download progress snapshot emitted over the `update-download-progress` event. */
export interface UpdateProgress {
  downloaded_bytes: number;
  total_bytes: number;
  percent: number;
}

/** Sidecar status snapshot returned by the `get_sidecar_statuses` IPC command. */
export interface SidecarStatusView {
  name: string;
  active: boolean;
}

/** Scheduler preferences returned by the `get_update_prefs` IPC command. */
export interface UpdatePrefs {
  auto_check_enabled: boolean;
  last_check_epoch: number;
  tgproxy_port?: number;
  tgproxy_secret?: string;
  tgproxy_enabled?: boolean;}

export interface DoctorStep {
  id: string;
  title: string;
  name?: string;
  status: "ok" | "fixed" | "failed" | "warn" | "skipped" | string;
  details: string;
  message?: string;
}

export interface DoctorReport {
  success: boolean;
  steps: DoctorStep[];
  message: string;
  summary?: string;
}

export interface DnsProvider {
  id: string;
  name: string;
  primary: string;
  secondary: string;
  description: string;
  is_doh_capable?: boolean;
}

export interface DnsStatus {
  adapter_name: string;
  current_dns: string[];
  active_servers?: string[];
  active_provider_id: string;
  current_provider?: string;
  available_providers: DnsProvider[];
  dns_scope?: "full" | "split";
  split_dns_active?: boolean;
  split_dns_servers?: string[];
}

export interface SmartHostsSourceInfo {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
}

export interface SmartHostsStatus {
  enabled: boolean;
  sources: SmartHostsSourceInfo[];
  include_adblock: boolean;
  last_update: number | null;
  entry_count: number;
  section_present: boolean;
  backup_present: boolean;
}

export interface SpeedtestResult {
  success: boolean;
  speed_mbps: number;
  mbps?: number;
  duration_ms: number;
  download_time_ms?: number;
  bytes_downloaded: number;
  resolution_tier: string;
  quality_tier?: string;
  message: string;
  verdict?: string;
}

export interface ListSyncItem {
  filename: string;
  lines_count: number;
  status: "updated" | "failed" | "created" | string;
}

export interface ListSyncReport {
  success: boolean;
  files_updated?: number;
  total_domains?: number;
  details?: string[];
  files: ListSyncItem[];
  message: string;
}

/** Camouflage transport registry entry (IPC `list_transports` snapshot). */
export interface TransportView {
  id: string;
  name: string;
  description: string;
  experimental: boolean;
  requires_exit_infra: boolean;
  available: boolean;
  unavailable_reason: string | null;
  default_config: TransportConfig;
  /** Example JSON for `options` (from the backend descriptor); documentation for researchers. */
  config_example: unknown;
}

/** Per-transport JSON config (stored in AppSettings.transport_configs). */
export interface TransportConfig {
  transport_id: string;
  enabled: boolean;
  /** Transport-specific options; validated by the transport itself. */
  options: unknown;
}
