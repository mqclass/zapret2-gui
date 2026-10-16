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

export interface StrategyBenchmarkResult {
  strategy_id: string;
  strategy_name: string;
  working: boolean;
  average_ping_ms: number | null;
  youtube_status: ProbeStatus;
  discord_status: ProbeStatus;
  summary: string;
}

export interface SupervisorStatus {
  state: SupervisorState;
  pid: number | null;
  active_strategy: string | null;
  binary_path: string | null;
  uptime_secs: number;
  total_logs_buffered: number;
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
  level: "stdout" | "stderr" | "info" | "warn" | "error";
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
}

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

