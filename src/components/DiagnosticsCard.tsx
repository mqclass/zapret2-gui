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
  RefreshCw,
  Search,
  Sparkles,
  Zap,
  Loader2,
  X,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { DiagnosticsReport, ProbeMetric } from "../types";
import { NavTab } from "./Sidebar";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";
import { useAutoTune } from "./AutoTuneModal";
import { ProbeStatusBadge } from "./ProbeStatusBadge";

// The candidate list moved to AutoTuneModal.tsx with the cycle itself;
// re-exported so the watchdog's import path stays unchanged.
export { AUTO_TUNE_CANDIDATES } from "./AutoTuneModal";

interface DiagnosticsCardProps {
  report: DiagnosticsReport;
  onRefresh?: () => void;
  isRefreshing?: boolean;
  onSelectAndStartStrategy?: (strategyId: string) => void;
  onNavigateTab?: (tab: NavTab) => void;
}

export const DiagnosticsCard: React.FC<DiagnosticsCardProps> = React.memo(
  ({ report, onRefresh, isRefreshing, onSelectAndStartStrategy, onNavigateTab }) => {
    const { t } = useI18n();

    // Custom Target Ping State
    const [customTarget, setCustomTarget] = useState("");
    const [isPingingCustom, setIsPingingCustom] = useState(false);
    const [customMetric, setCustomMetric] = useState<ProbeMetric | null>(null);

    // Visible error for user-initiated actions (previously only console.error)
    const [actionError, setActionError] = useState<string | null>(null);

    // Auto-tune cycle + results modal: shared with the dashboard via
    // useAutoTune; this card only hosts the header button and the
    // action-error banner the cycle reports into.
    const { isAutoTuning, startAutoTune, autoTuneModal } = useAutoTune({
      onSelectAndStartStrategy,
      onNavigateTab,
      onError: setActionError,
    });

    const handlePingCustom = useCallback(async (e: React.FormEvent) => {
      e.preventDefault();
      const trimmed = customTarget.trim();
      if (!trimmed) return;

      setIsPingingCustom(true);
      setActionError(null);
      try {
        const metric = await invoke<ProbeMetric>("ping_custom_target", { target: trimmed });
        setCustomMetric(metric);
      } catch (err) {
        console.error("Custom ping error:", err);
        setActionError(formatErrorMessage(err));
      } finally {
        setIsPingingCustom(false);
      }
    }, [customTarget]);

    const renderMetricCard = (
      title: string,
      subtitle: string,
      metric: ProbeMetric | undefined,
      icon: React.ReactNode,
      iconBg: string,
      idx: number
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
        <div
          className="t-card flex flex-col justify-between p-3.5 hover-lift hover:border-linestrong/80 stagger-item"
          style={{ "--stagger-i": idx } as React.CSSProperties}
        >
          <div>
            <div className="flex items-start gap-2 mb-2.5">
              <div className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-control ${iconBg}`}>
                {icon}
              </div>
              <div className="min-w-0">
                <div className="text-xs font-semibold text-ink leading-snug line-clamp-2">{title}</div>
                <div className="text-[10px] text-ink4 leading-snug line-clamp-2">{subtitle}</div>
              </div>
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

          <div className="mt-2 flex items-center justify-between gap-2">
            <span className="text-[10px] text-ink4 truncate" title={m.message}>
              {m.message}
            </span>
            <span className="shrink-0"><ProbeStatusBadge status={m.status} /></span>
          </div>
        </div>
      );
    };

    return (
      <div className="space-y-3">
        {actionError && (
          <div className="flex items-start gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-rose-400" />
            <span className="leading-relaxed">{actionError}</span>
            <button
              type="button"
              onClick={() => setActionError(null)}
              className="ml-auto shrink-0 text-rose-400 hover:text-rose-200 cursor-pointer"
              aria-label={t.doctor.closeBtn}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}
        {/* Top Header with Auto-Tune and Refresh */}
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Activity className="h-4 w-4 text-accent" />
            <h2 className="text-sm font-semibold text-ink">{t.diagnostics.cardTitle}</h2>
            {onRefresh && (
              <button
                type="button"
                onClick={onRefresh}
                disabled={isRefreshing}
                title={t.diagnostics.refreshTooltip}
                className="p-1 rounded-chip text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isRefreshing ? "animate-spin text-accent" : ""}`} />
              </button>
            )}
          </div>

          <button
            type="button"
            onClick={startAutoTune}
            disabled={isAutoTuning}
            className="flex items-center gap-1.5 rounded-control border border-accent-line bg-accent-wash px-2.5 py-1 text-xs font-semibold text-accent-soft hover:bg-accent-fill/20 hover:text-ink transition-all cursor-pointer shadow-sm shadow-accent/10 disabled:opacity-50"
          >
            {isAutoTuning ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Sparkles className="h-3.5 w-3.5 text-accent" />
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
            "bg-red-500/15",
            0
          )}
          {renderMetricCard(
            t.diagnostics.discordTitle,
            t.diagnostics.discordSubtitle,
            report.discord,
            <MessageSquare className="h-4 w-4 text-indigo-400" />,
            "bg-indigo-500/15",
            1
          )}
          {renderMetricCard(
            t.diagnostics.discordVoiceTitle,
            t.diagnostics.discordVoiceSubtitle,
            report.discord_voice,
            <Volume2 className="h-4 w-4 text-violet-400" />,
            "bg-violet-500/15",
            2
          )}
          {renderMetricCard(
            t.diagnostics.telegramTitle,
            t.diagnostics.telegramSubtitle,
            report.telegram,
            <Send className="h-4 w-4 text-sky-400" />,
            "bg-sky-500/15",
            3
          )}
          {renderMetricCard(
            t.diagnostics.twitterTitle,
            t.diagnostics.twitterSubtitle,
            report.twitter,
            <Twitter className="h-4 w-4 text-accent" />,
            "bg-accent-wash",
            4
          )}
          {renderMetricCard(
            t.diagnostics.cloudflareTitle,
            t.diagnostics.cloudflareSubtitle,
            report.cloudflare,
            <Globe className="h-4 w-4 text-amber-400" />,
            "bg-amber-500/15",
            5
          )}
        </div>

        {/* Custom Target Latency & DPI Probe */}
        <div className="rounded-panel border border-line bg-cardin p-3.5">
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
                className="h-8 w-full rounded-control border border-line bg-cardin pl-8 pr-3 text-xs text-ink2 placeholder-ink5 focus:border-accent-line focus:outline-none"
              />
            </div>
            <button
              type="submit"
              disabled={isPingingCustom || !customTarget.trim()}
              className="flex h-8 items-center gap-1.5 rounded-control bg-accent-fill px-3 text-xs font-semibold text-ink hover:bg-accent transition-colors disabled:opacity-50 cursor-pointer"
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
            <div className="mt-2.5 rounded-control border border-line bg-cardin/70 p-3 text-xs">
              <div className="flex items-center justify-between mb-2">
                <span className="font-semibold text-ink">{customMetric.target}</span>
                <ProbeStatusBadge status={customMetric.status} />
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-[11px] border-t border-raise/80 pt-2">
                <div>
                  <span className="text-ink5 block text-[10px]">{t.diagnostics.latency}</span>
                  <span className="font-mono font-medium text-accent-soft">
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

        {/* Auto-tune results modal + classifier (shared hook) */}
        {autoTuneModal}
      </div>
    );
  }
);
