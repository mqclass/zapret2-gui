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
