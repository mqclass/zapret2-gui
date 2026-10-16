import React, { useState, useCallback } from "react";
import {
  Youtube,
  MessageSquare,
  Volume2,
  Send,
  Twitter,
  Globe,
  Activity,
  AlertCircle,
  CheckCircle2,
  XCircle,
  RefreshCw,
  Search,
  Sparkles,
  Zap,
  ArrowRight,
  ShieldAlert,
  Loader2,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { DiagnosticsReport, ProbeMetric, ProbeStatus, StrategyBenchmarkResult } from "../types";
import { useI18n } from "../i18n";

interface DiagnosticsCardProps {
  report: DiagnosticsReport;
  onRefresh?: () => void;
  isRefreshing?: boolean;
  onSelectAndStartStrategy?: (strategyId: string) => void;
}

export const DiagnosticsCard: React.FC<DiagnosticsCardProps> = React.memo(
  ({ report, onRefresh, isRefreshing, onSelectAndStartStrategy }) => {
    const { t } = useI18n();

    // Custom Target Ping State
    const [customTarget, setCustomTarget] = useState("");
    const [isPingingCustom, setIsPingingCustom] = useState(false);
    const [customMetric, setCustomMetric] = useState<ProbeMetric | null>(null);

    // Auto-Tune Benchmark State
    const [isAutoTuning, setIsAutoTuning] = useState(false);
    const [benchmarkResults, setBenchmarkResults] = useState<StrategyBenchmarkResult[] | null>(null);
    const [isAutoTuneModalOpen, setIsAutoTuneModalOpen] = useState(false);

    const handlePingCustom = useCallback(async (e: React.FormEvent) => {
      e.preventDefault();
      const trimmed = customTarget.trim();
      if (!trimmed) return;

      setIsPingingCustom(true);
      try {
        const metric = await invoke<ProbeMetric>("ping_custom_target", { target: trimmed });
        setCustomMetric(metric);
      } catch (err) {
        console.error("Custom ping error:", err);
      } finally {
        setIsPingingCustom(false);
      }
    }, [customTarget]);

    const handleAutoTune = useCallback(async () => {
      setIsAutoTuning(true);
      try {
        const results = await invoke<StrategyBenchmarkResult[]>("benchmark_strategies");
        setBenchmarkResults(results);
        setIsAutoTuneModalOpen(true);
      } catch (err) {
        console.error("Benchmark error:", err);
      } finally {
        setIsAutoTuning(false);
      }
    }, []);

    const getStatusBadge = (status: ProbeStatus) => {
      switch (status) {
        case "operational":
          return (
            <span className="flex items-center gap-1 rounded-full bg-emerald-500/15 border border-emerald-500/30 px-2 py-0.5 text-[11px] font-semibold text-emerald-400">
              <CheckCircle2 className="h-3 w-3" />
              {t.diagnostics.operational}
            </span>
          );
        case "degraded":
          return (
            <span className="flex items-center gap-1 rounded-full bg-amber-500/15 border border-amber-500/30 px-2 py-0.5 text-[11px] font-semibold text-amber-400">
              <AlertCircle className="h-3 w-3" />
              {t.diagnostics.degraded}
            </span>
          );
        case "blocked":
          return (
            <span className="flex items-center gap-1 rounded-full bg-rose-500/15 border border-rose-500/30 px-2 py-0.5 text-[11px] font-semibold text-rose-400">
              <XCircle className="h-3 w-3" />
              {t.diagnostics.blocked}
            </span>
          );
        case "checking":
        default:
          return (
            <span className="flex items-center gap-1 rounded-full bg-ink5/15 border border-ink5/30 px-2 py-0.5 text-[11px] font-semibold text-ink4">
              <Activity className="h-3 w-3 animate-pulse text-cyan-400" />
              {t.diagnostics.checking}
            </span>
          );
      }
    };

    const renderMetricCard = (
      title: string,
      subtitle: string,
      metric: ProbeMetric | undefined,
      icon: React.ReactNode,
      iconBg: string
    ) => {
      const m = metric || {
        target: title,
        status: "checking",
        latency_ms: null,
        dns_ms: null,
        tcp_ms: null,
        http_status: null,
        packet_loss_pct: 0,
        last_checked_epoch: 0,
        ip_address: null,
        message: t.diagnostics.checking,
      };

      return (
        <div className="flex flex-col justify-between rounded-panel border border-raise bg-card/40 p-3.5 transition-all hover:border-linestrong/80">
          <div>
            <div className="flex items-start justify-between gap-2 mb-2.5">
              <div className="flex items-center gap-2 min-w-0">
                <div className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-control ${iconBg}`}>
                  {icon}
                </div>
                <div className="min-w-0">
                  <div className="text-xs font-semibold text-ink truncate">{title}</div>
                  <div className="text-[10px] text-ink4 truncate">{subtitle}</div>
                </div>
              </div>
              <div className="shrink-0">{getStatusBadge(m.status)}</div>
            </div>

            {/* Metrics Breakdown Grid */}
            <div className="grid grid-cols-3 gap-2 border-t border-raise/80 pt-2 text-[11px]">
              <div>
                <span className="text-ink5 block text-[10px]">{t.diagnostics.latency}</span>
                <span className="font-mono font-medium text-ink2">
                  {m.latency_ms !== null && m.latency_ms !== undefined ? `${m.latency_ms} ms` : "—"}
                </span>
              </div>
              <div>
                <span className="text-ink5 block text-[10px]">{t.diagnostics.dnsLatency}</span>
                <span className="font-mono font-medium text-ink3">
                  {m.dns_ms !== null && m.dns_ms !== undefined ? `${m.dns_ms} ms` : "—"}
                </span>
              </div>
              <div>
                <span className="text-ink5 block text-[10px]">{t.diagnostics.tcpLatency}</span>
                <span className="font-mono font-medium text-ink3">
                  {m.tcp_ms !== null && m.tcp_ms !== undefined ? `${m.tcp_ms} ms` : "—"}
                </span>
              </div>
            </div>

            <div className="mt-1.5 flex items-center justify-between text-[10px] text-ink4">
              <span>
                {m.http_status ? `HTTP ${m.http_status}` : ""} {m.ip_address ? `• ${m.ip_address}` : ""}
              </span>
              {m.packet_loss_pct > 0 && (
                <span className="text-rose-400 font-medium">Loss: {m.packet_loss_pct}%</span>
              )}
            </div>
          </div>

          <div className="mt-2 text-[10px] text-ink4 truncate" title={m.message}>
            {m.message}
          </div>
        </div>
      );
    };

    return (
      <div className="space-y-3">
        {/* Top Header with Auto-Tune and Refresh */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Activity className="h-4 w-4 text-cyan-400" />
            <h2 className="text-sm font-semibold text-ink">Диагностика сети и блокировок</h2>
            {onRefresh && (
              <button
                type="button"
                onClick={onRefresh}
                disabled={isRefreshing}
                title={t.diagnostics.refreshTooltip}
                className="p-1 rounded-chip text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isRefreshing ? "animate-spin text-cyan-400" : ""}`} />
              </button>
            )}
          </div>

          <button
            type="button"
            onClick={handleAutoTune}
            disabled={isAutoTuning}
            className="flex items-center gap-1.5 rounded-control border border-cyan-500/40 bg-cyan-500/10 px-2.5 py-1 text-xs font-semibold text-cyan-300 hover:bg-cyan-500/20 hover:text-ink transition-all cursor-pointer shadow-sm shadow-cyan-500/10 disabled:opacity-50"
          >
            {isAutoTuning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Sparkles className="h-3.5 w-3.5 text-cyan-400" />
            )}
            <span>{isAutoTuning ? t.diagnostics.autoTuneRunning : t.diagnostics.autoTuneBtn}</span>
          </button>
        </div>

        {/* 6 Monitored Target Cards */}
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
          {renderMetricCard(
            t.diagnostics.youtubeTitle,
            t.diagnostics.youtubeSubtitle,
            report.youtube,
            <Youtube className="h-4 w-4 text-red-400" />,
            "bg-red-500/15"
          )}
          {renderMetricCard(
            t.diagnostics.discordTitle,
            t.diagnostics.discordSubtitle,
            report.discord,
            <MessageSquare className="h-4 w-4 text-indigo-400" />,
            "bg-indigo-500/15"
          )}
          {renderMetricCard(
            t.diagnostics.discordVoiceTitle,
            t.diagnostics.discordVoiceSubtitle,
            report.discord_voice,
            <Volume2 className="h-4 w-4 text-violet-400" />,
            "bg-violet-500/15"
          )}
          {renderMetricCard(
            t.diagnostics.telegramTitle,
            t.diagnostics.telegramSubtitle,
            report.telegram,
            <Send className="h-4 w-4 text-sky-400" />,
            "bg-sky-500/15"
          )}
          {renderMetricCard(
            t.diagnostics.twitterTitle,
            t.diagnostics.twitterSubtitle,
            report.twitter,
            <Twitter className="h-4 w-4 text-accent" />,
            "bg-accent-wash"
          )}
          {renderMetricCard(
            t.diagnostics.cloudflareTitle,
            t.diagnostics.cloudflareSubtitle,
            report.cloudflare,
            <Globe className="h-4 w-4 text-amber-400" />,
            "bg-amber-500/15"
          )}
        </div>

        {/* Custom Target Latency & DPI Probe */}
        <div className="rounded-panel border border-raise bg-card/30 p-3.5">
          <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
            <div>
              <div className="text-xs font-semibold text-ink2">{t.diagnostics.customPingTitle}</div>
              <div className="text-[10px] text-ink4">{t.diagnostics.customPingSubtitle}</div>
            </div>
          </div>

          <form onSubmit={handlePingCustom} className="flex gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-ink5" />
              <input
                type="text"
                value={customTarget}
                onChange={(e) => setCustomTarget(e.target.value)}
                placeholder={t.diagnostics.customPingPlaceholder}
                className="h-8 w-full rounded-control border border-raise bg-cardin pl-8 pr-3 text-xs text-ink2 placeholder-ink5 focus:border-cyan-500 focus:outline-none"
              />
            </div>
            <button
              type="submit"
              disabled={isPingingCustom || !customTarget.trim()}
              className="flex h-8 items-center gap-1.5 rounded-control bg-cyan-600 px-3 text-xs font-semibold text-ink hover:bg-cyan-500 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isPingingCustom ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Zap className="h-3.5 w-3.5" />
              )}
              <span>{isPingingCustom ? t.diagnostics.customPingTesting : t.diagnostics.customPingBtn}</span>
            </button>
          </form>

          {/* Custom Ping Result Box */}
          {customMetric && (
            <div className="mt-2.5 rounded-control border border-raise bg-cardin/70 p-3 text-xs">
              <div className="flex items-center justify-between mb-2">
                <span className="font-semibold text-ink">{customMetric.target}</span>
                {getStatusBadge(customMetric.status)}
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-[11px] border-t border-raise/80 pt-2">
                <div>
                  <span className="text-ink5 block text-[10px]">{t.diagnostics.latency}</span>
                  <span className="font-mono font-medium text-cyan-300">
                    {customMetric.latency_ms !== null ? `${customMetric.latency_ms} ms` : "—"}
                  </span>
                </div>
                <div>
                  <span className="text-ink5 block text-[10px]">{t.diagnostics.dnsLatency}</span>
                  <span className="font-mono font-medium text-ink3">
                    {customMetric.dns_ms !== null ? `${customMetric.dns_ms} ms` : "—"}
                  </span>
                </div>
                <div>
                  <span className="text-ink5 block text-[10px]">{t.diagnostics.tcpLatency}</span>
                  <span className="font-mono font-medium text-ink3">
                    {customMetric.tcp_ms !== null ? `${customMetric.tcp_ms} ms` : "—"}
                  </span>
                </div>
                <div>
                  <span className="text-ink5 block text-[10px]">{t.diagnostics.ipAddress}</span>
                  <span className="font-mono text-ink3">
                    {customMetric.ip_address || "—"}
                  </span>
                </div>
              </div>
              <div className="mt-2 text-[11px] text-ink3">
                {customMetric.message}
              </div>
            </div>
          )}
        </div>

        {/* Auto-Tune Results Modal */}
        {isAutoTuneModalOpen && benchmarkResults && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-overlay backdrop-blur-md p-4">
            <div className="w-full max-w-lg rounded-card border border-raise bg-card p-5 shadow-2xl">
              <div className="flex items-center justify-between pb-3 border-b border-raise">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-5 w-5 text-cyan-400" />
                  <h3 className="text-sm font-semibold text-ink">{t.diagnostics.autoTuneModalTitle}</h3>
                </div>
                <button
                  type="button"
                  onClick={() => setIsAutoTuneModalOpen(false)}
                  className="rounded-control p-1 text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer"
                >
                  ✕
                </button>
              </div>

              <div className="mt-3 text-xs text-ink3">
                {t.diagnostics.autoTuneDesc}
              </div>

              <div className="mt-3 space-y-2 max-h-72 overflow-y-auto pr-1">
                {benchmarkResults.map((res) => (
                  <div
                    key={res.strategy_id}
                    className={`rounded-panel border p-3 text-xs transition-all ${
                      res.working
                        ? "border-emerald-500/40 bg-emerald-950/20 hover:border-emerald-500"
                        : "border-raise bg-cardin/40 opacity-70"
                    }`}
                  >
                    <div className="flex items-center justify-between mb-1.5">
                      <span className="font-semibold text-ink">{res.strategy_name}</span>
                      <div className="flex items-center gap-2">
                        {res.average_ping_ms && (
                          <span className="font-mono text-[11px] text-cyan-400">
                            ~{res.average_ping_ms} ms
                          </span>
                        )}
                        {getStatusBadge(res.youtube_status)}
                      </div>
                    </div>
                    <p className="text-[11px] text-ink4">{res.summary}</p>
                    {res.working && onSelectAndStartStrategy && (
                      <button
                        type="button"
                        onClick={() => {
                          onSelectAndStartStrategy(res.strategy_id);
                          setIsAutoTuneModalOpen(false);
                        }}
                        className="mt-2.5 flex items-center gap-1 rounded-control bg-emerald-600/30 border border-emerald-500/50 px-2.5 py-1 text-[11px] font-semibold text-emerald-300 hover:bg-emerald-600/50 cursor-pointer transition-colors"
                      >
                        <Zap className="h-3 w-3" />
                        <span>{t.diagnostics.autoTuneApply}</span>
                      </button>
                    )}
                  </div>
                ))}
              </div>

              <div className="mt-4 flex justify-end">
                <button
                  type="button"
                  onClick={() => setIsAutoTuneModalOpen(false)}
                  className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
                >
                  Закрыть
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    );
  }
);
