import React, { useState, useCallback, useRef } from "react";
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
  X,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { DiagnosticsReport, ProbeMetric, ProbeStatus, StrategyProbeResult, SupervisorStatusResponse } from "../types";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";

// Candidates probed in order. Custom user profiles are out of scope for
// the first slice: these cover the shipped engines. The three new winws2
// lua presets (yt4k, discord-light, tspu-max) are included; sites-light
// is deliberately left out: the weakest tier cannot win YouTube/Discord
// probes, it is a manual pick for weak hardware.
const AUTO_TUNE_CANDIDATES = [
  "flowseal-alt",
  "flowseal-alt11",
  "flowseal-fake-tls-auto",
  "zapret2-ru-ultimate",
  "flowseal-general",
  "yt4k",
  "discord-light",
  "tspu-max",
];

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

    // Auto-Tune State: real per-strategy probing. The engine restarts on
    // each candidate, so connections drop during the run and the original
    // state is restored at the end (even on cancel or error).
    const [isAutoTuning, setIsAutoTuning] = useState(false);
    const [autoTuneProgress, setAutoTuneProgress] = useState<{ index: number; total: number; id: string } | null>(null);
    const [probeResults, setProbeResults] = useState<StrategyProbeResult[] | null>(null);
    const [isAutoTuneModalOpen, setIsAutoTuneModalOpen] = useState(false);
    const autoTuneCancelRef = useRef(false);
    const autoTuneRunningRef = useRef(false);

    // Visible error for user-initiated actions (previously only console.error)
    const [actionError, setActionError] = useState<string | null>(null);

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

    const handleAutoTune = useCallback(async () => {
      // Ref guard, set synchronously: the state guard alone is stale until
      // re-render, so a double click before then would start two loops
      // whose probes interleave and get attributed to the wrong strategy.
      if (autoTuneRunningRef.current) return;
      autoTuneRunningRef.current = true;
      autoTuneCancelRef.current = false;
      setActionError(null);

      // Pre-check: read the starting state for the restore step. Starting /
      // Stopping are waited out (up to ~5s); anything else aborts, since a
      // probe started mid-transition would measure the wrong engine.
      // No elevation refusal here: with a running engine probing works
      // (elevated GUI or attached svc), and without elevation the first
      // probe fails fast with ADMIN_REQUIRED, which aborts the loop below.
      let wasRunning = false;
      let originalId: string | null = null;
      try {
        let st = await invoke<SupervisorStatusResponse>("get_supervisor_status");
        for (
          let i = 0;
          i < 10 && (st.supervisor.state === "starting" || st.supervisor.state === "stopping");
          i++
        ) {
          await new Promise((r) => setTimeout(r, 500));
          st = await invoke<SupervisorStatusResponse>("get_supervisor_status");
        }
        if (st.supervisor.state === "starting" || st.supervisor.state === "stopping") {
          setActionError(t.diagnostics.autoTuneBusy);
          autoTuneRunningRef.current = false;
          return;
        }
        wasRunning = st.supervisor.state === "running";
        originalId = st.supervisor.active_strategy ?? null;
      } catch (err) {
        console.error("Auto-tune pre-check error:", err);
        setActionError(formatErrorMessage(err));
        autoTuneRunningRef.current = false;
        return;
      }

      setIsAutoTuning(true);
      setProbeResults([]);
      setIsAutoTuneModalOpen(true);
      const results: StrategyProbeResult[] = [];
      let probedAny = false;
      let lastStartedId: string | null = null;
      try {
        for (let i = 0; i < AUTO_TUNE_CANDIDATES.length; i++) {
          if (autoTuneCancelRef.current) break;
          const id = AUTO_TUNE_CANDIDATES[i];
          setAutoTuneProgress({ index: i + 1, total: AUTO_TUNE_CANDIDATES.length, id });
          probedAny = true;
          lastStartedId = id;
          try {
            const res = await invoke<StrategyProbeResult>("probe_strategy", { strategyId: id });
            results.push(res);
            setProbeResults([...results]);
            if (res.error && res.error.includes("ADMIN_REQUIRED")) {
              setActionError(t.adminGate.message);
              break;
            }
          } catch (err) {
            console.error("Probe error:", err);
            results.push({
              strategy_id: id,
              strategy_name: id,
              youtube_status: "blocked",
              youtube_ms: null,
              discord_status: "blocked",
              discord_ms: null,
              voice_status: "blocked",
              voice_ms: null,
              score: 0,
              average_ping_ms: null,
              engine_running: false,
              error: formatErrorMessage(err),
            });
            setProbeResults([...results]);
          }
        }
      } finally {
        // Restore only what the loop itself left behind. The user may have
        // intervened while a probe was in flight (started or stopped the
        // engine by hand): re-read the state and never override their choice.
        if (probedAny) {
          try {
            const current = await invoke<SupervisorStatusResponse>("get_supervisor_status");
            const curState = current.supervisor.state;
            const curStrategy = current.supervisor.active_strategy ?? null;
            if (wasRunning && originalId) {
              if (
                curState === "running" &&
                curStrategy === lastStartedId &&
                lastStartedId !== originalId
              ) {
                onSelectAndStartStrategy?.(originalId);
              }
              // Already on the original, or the user stopped/switched: leave it.
            } else if (!wasRunning) {
              if (curState === "running" && curStrategy === lastStartedId) {
                try {
                  await invoke("stop_supervisor");
                } catch (err) {
                  console.error("Auto-tune restore stop error:", err);
                }
                try {
                  await invoke("set_last_strategy", { strategyId: originalId });
                } catch (err) {
                  console.error("Auto-tune restore settings error:", err);
                }
              } else if (curState === "stopped" || curState === "stopping") {
                // Engine already down (by the user or by itself): only the
                // persisted last strategy still points at a probed one.
                try {
                  await invoke("set_last_strategy", { strategyId: originalId });
                } catch (err) {
                  console.error("Auto-tune restore settings error:", err);
                }
              }
              // Running on another strategy or starting: the user took over.
            }
            // Running with no recorded strategy: leave the engine as-is
            // rather than guessing a strategy to restore.
          } catch (err) {
            console.error("Auto-tune restore error:", err);
            setActionError(formatErrorMessage(err));
          }
        }
        setIsAutoTuning(false);
        setAutoTuneProgress(null);
        autoTuneRunningRef.current = false;
      }
    }, [onSelectAndStartStrategy, t]);

    const handleCancelAutoTune = useCallback(() => {
      autoTuneCancelRef.current = true;
      setIsAutoTuneModalOpen(false);
    }, []);

    const sortedProbeResults = probeResults
      ? [...probeResults].sort(
          (a, b) =>
            b.score - a.score ||
            (a.average_ping_ms ?? Number.MAX_SAFE_INTEGER) -
              (b.average_ping_ms ?? Number.MAX_SAFE_INTEGER)
        )
      : [];
    const autoTuneWinner =
      sortedProbeResults.length > 0 &&
      sortedProbeResults[0].score > 0 &&
      sortedProbeResults[0].engine_running
        ? sortedProbeResults[0]
        : null;

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
              <Activity className="h-3 w-3 animate-pulse text-accent" />
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
            <span className="shrink-0">{getStatusBadge(m.status)}</span>
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
            onClick={handleAutoTune}
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
                {getStatusBadge(customMetric.status)}
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

        {/* Auto-Tune Results Modal */}
        {isAutoTuneModalOpen && (isAutoTuning || probeResults) && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-overlay backdrop-blur-md p-4 modal-backdrop-enter">
            <div className="w-full max-w-lg rounded-card border border-line bg-card p-5 shadow-2xl modal-panel-enter">
              <div className="flex items-center justify-between pb-3 border-b border-raise">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-5 w-5 text-accent" />
                  <h3 className="text-sm font-semibold text-ink">{t.diagnostics.autoTuneModalTitle}</h3>
                </div>
                <button
                  type="button"
                  onClick={isAutoTuning ? handleCancelAutoTune : () => setIsAutoTuneModalOpen(false)}
                  className="rounded-control p-1 text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              {isAutoTuning && autoTuneProgress ? (
                <div className="mt-4 space-y-3">
                  <div className="flex items-center gap-2 text-xs text-ink2">
                    <Loader2 className="h-4 w-4 animate-spin text-accent" />
                    <span>
                      {t.diagnostics.autoTuneTesting} {autoTuneProgress.index}/{autoTuneProgress.total}:{" "}
                      {autoTuneProgress.id}
                    </span>
                  </div>
                  <button
                    type="button"
                    onClick={handleCancelAutoTune}
                    className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
                  >
                    {t.diagnostics.autoTuneCancel}
                  </button>
                </div>
              ) : (
                <>
                  <div className="mt-3 text-xs text-ink3">{t.diagnostics.autoTuneDesc}</div>

                  {autoTuneWinner && onSelectAndStartStrategy && (
                    <button
                      type="button"
                      onClick={() => {
                        onSelectAndStartStrategy(autoTuneWinner.strategy_id);
                        setIsAutoTuneModalOpen(false);
                      }}
                      className="mt-3 flex items-center gap-1 rounded-control bg-emerald-600/30 border border-emerald-500/50 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-600/50 cursor-pointer transition-colors"
                    >
                      <Zap className="h-3 w-3" />
                      <span>
                        {t.diagnostics.autoTuneApplyBest}: {autoTuneWinner.strategy_name}
                      </span>
                    </button>
                  )}

                  <div className="mt-3 space-y-2 max-h-72 overflow-y-auto pr-1">
                    {sortedProbeResults.length === 0 && (
                      <p className="text-xs text-ink4">{t.diagnostics.autoTuneNoWinner}</p>
                    )}
                    {sortedProbeResults.map((res) => {
                      const isWinner = autoTuneWinner?.strategy_id === res.strategy_id;
                      return (
                        <div
                          key={res.strategy_id}
                          className={`rounded-panel border p-3 text-xs transition-all ${
                            isWinner
                              ? "border-emerald-500/60 bg-emerald-950/30"
                              : res.score > 0
                                ? "border-raise bg-cardin/40"
                                : "border-raise bg-cardin/40 opacity-70"
                          }`}
                        >
                          <div className="flex items-center justify-between mb-1.5">
                            <span className="font-semibold text-ink">
                              {res.strategy_name}
                              {isWinner && (
                                <span className="ml-2 rounded-full bg-emerald-500/20 border border-emerald-500/40 px-2 py-0.5 text-[10px] font-semibold text-emerald-300">
                                  {t.diagnostics.autoTuneWinner}
                                </span>
                              )}
                            </span>
                            {res.average_ping_ms !== null && (
                              <span className="font-mono text-[11px] text-accent">
                                ~{res.average_ping_ms} ms
                              </span>
                            )}
                          </div>
                          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                            <span className="flex items-center gap-1 text-ink4">
                              YouTube {getStatusBadge(res.youtube_status)}
                            </span>
                            <span className="flex items-center gap-1 text-ink4">
                              Discord {getStatusBadge(res.discord_status)}
                            </span>
                            <span className="flex items-center gap-1 text-ink4">
                              Voice {getStatusBadge(res.voice_status)}
                            </span>
                          </div>
                          {res.error && (
                            <p className="mt-1.5 text-[11px] text-rose-400">
                              {t.diagnostics.autoTuneEngineFailed}: {res.error}
                            </p>
                          )}
                          {!res.error && !res.engine_running && (
                            <p className="mt-1.5 text-[11px] text-rose-400">
                              {t.diagnostics.autoTuneEngineFailed}
                            </p>
                          )}
                          {res.score > 0 && res.engine_running && onSelectAndStartStrategy && (
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
                      );
                    })}
                  </div>

                  <div className="mt-4 flex justify-end">
                    <button
                      type="button"
                      onClick={() => setIsAutoTuneModalOpen(false)}
                      className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
                    >
                      {t.speedtest.closeBtn}
                    </button>
                  </div>
                </>
              )}
            </div>
          </div>
        )}
      </div>
    );
  }
);
