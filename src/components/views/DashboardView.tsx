import React from "react";
import { motion } from "framer-motion";
import {
  Shield,
  ShieldCheck,
  Play,
  Square,
  Loader2,
  Zap,
  Globe2,
  Tv,
  Wrench,
  Mic,
  ChevronRight,
  AlertTriangle,
} from "lucide-react";
import {
  SupervisorStatus,
  DiagnosticsReport,
  LuaStrategy,
  SplitTunnelMode,
  UpdateInfo,
} from "../../types";
import { useI18n } from "../../i18n";
import { SplitTunnelBar } from "../SplitTunnelBar";
import { HeroOrb, OrbState } from "../HeroOrb";
import { Sparkline } from "../Sparkline";
import { UpdateBanner } from "../UpdateBanner";
import { NavTab } from "../Sidebar";

interface DashboardViewProps {
  status: SupervisorStatus;
  diagnostics: DiagnosticsReport;
  strategies: LuaStrategy[];
  selectedStrategyId: string;
  splitTunnelMode: SplitTunnelMode;
  onModeChange: (mode: SplitTunnelMode) => void;
  onStart: () => void;
  onStop: () => void;
  onSelectStrategy: (id: string) => void;
  onStartStrategy: (id: string) => void;
  onRefreshProbes: () => void;
  isRefreshingProbes: boolean;
  onNavigateTab: (tab: NavTab) => void;
  updateInfo: UpdateInfo | null;
  onOpenUpdateDialog: () => void;
  onDismissUpdate: () => void;
}

export const DashboardView: React.FC<DashboardViewProps> = React.memo(
  ({
    status,
    diagnostics,
    strategies,
    selectedStrategyId,
    splitTunnelMode,
    onModeChange,
    onStart,
    onStop,
    onSelectStrategy,
    onStartStrategy,
    onRefreshProbes,
    isRefreshingProbes,
    onNavigateTab,
    updateInfo,
    onOpenUpdateDialog,
    onDismissUpdate,
  }) => {
    const { t } = useI18n();

    // Rolling ping history for sparklines (real probe data, 40 samples).
    // Keyed by diagnostics.timestamp so re-renders without new probe data
    // don't push duplicates.
    const pingHist = React.useRef<{ ts: number; data: Record<string, (number | null)[]> }>({
      ts: -1,
      data: {},
    });
    if (pingHist.current.ts !== diagnostics.timestamp) {
      pingHist.current.ts = diagnostics.timestamp;
      for (const [key, v] of [
        ["yt", diagnostics.youtube.latency_ms],
        ["dc", diagnostics.discord.latency_ms],
      ] as const) {
        const h = pingHist.current.data[key] ?? [];
        h.push(v);
        if (h.length > 40) h.shift();
        pingHist.current.data[key] = h;
      }
    }
    const ytHist = pingHist.current.data["yt"] ?? [];
    const dcHist = pingHist.current.data["dc"] ?? [];

    const isRunning = status.state === "running";
    const isStarting = status.state === "starting";
    const isStopping = status.state === "stopping";
    const isTransitioning = isStarting || isStopping;
    const orbState: OrbState = isRunning
      ? "connected"
      : isTransitioning
      ? "connecting"
      : "idle";
    const orbLabel = isRunning
      ? t.dashboard.stopBypass
      : isTransitioning
      ? t.dashboard.pleaseWait
      : t.dashboard.startBypass;

    const activeStrat = strategies.find((s) => s.id === status.active_strategy);
    const selectedStrat = strategies.find((s) => s.id === selectedStrategyId);

    const formatUptime = (secs: number) => {
      const hrs = Math.floor(secs / 3600);
      const mins = Math.floor((secs % 3600) / 60);
      const s = secs % 60;
      const { uptimeH: h, uptimeM: m, uptimeS: sec } = t.dashboard;
      if (hrs > 0) return `${hrs}${h} ${mins}${m} ${s}${sec}`;
      if (mins > 0) return `${mins}${m} ${s}${sec}`;
      return `${s}${sec}`;
    };

    return (
      <motion.div
        className="space-y-4 pb-4"
        initial="hidden"
        animate="show"
        variants={{
          hidden: {},
          show: { transition: { staggerChildren: 0.08, delayChildren: 0.05 } },
        }}
      >
        {/* Update availability banner (announces only — no silent downloads) */}
        {updateInfo?.update_available && (
          <UpdateBanner
            info={updateInfo}
            onOpenDialog={onOpenUpdateDialog}
            onDismiss={onDismissUpdate}
          />
        )}
        {/* WinDivert conflict: the engine died with exit code 10 because the
            driver is held by another bypass tool. Shown until the next
            successful start clears it; hidden while running. */}
        {status.last_engine_error === "windivert_conflict" &&
          status.state !== "running" && (
            <div className="rounded-panel border border-amber-500/30 bg-amber-500/10 p-4 text-amber-200 backdrop-blur-md">
              <div className="flex items-start gap-2.5">
                <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-amber-400" />
                <div className="text-xs">
                  <div className="font-semibold text-amber-100">
                    {t.dashboard.windivertConflictTitle}
                  </div>
                  <div className="mt-0.5 leading-relaxed">
                    {t.dashboard.windivertConflictBody}
                  </div>
                </div>
              </div>
            </div>
          )}
        {/* Hero Control Card */}
        <motion.div
          variants={{
            hidden: { opacity: 0, y: 18 },
            show: {
              opacity: 1,
              y: 0,
              transition: { duration: 0.5, ease: [0.22, 1, 0.36, 1] },
            },
          }}
          className="t-card relative overflow-hidden p-6 md:p-8"
        >
          {/* ambient glow that warms up when the bypass is on */}
          <span
            aria-hidden
            className={`pointer-events-none absolute -right-16 -top-24 h-72 w-72 rounded-full blur-3xl transition-colors duration-1000 ${
              isRunning ? "bg-emerald-500/[0.13]" : "bg-accent/[0.09]"
            }`}
          />
          <span
            aria-hidden
            className="pointer-events-none absolute inset-x-10 top-0 h-px bg-gradient-to-r from-transparent via-accent-line to-transparent"
          />
          <div className="relative flex flex-col md:flex-row md:items-center justify-between gap-8">
            {/* Status and Active Info */}
            <div className="space-y-4 max-w-xl">
              <span
                className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-semibold tracking-wide uppercase ${
                  isRunning
                    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400 shadow-[0_0_20px_rgba(52,211,153,0.22)]"
                    : isTransitioning
                    ? "border-amber-500/30 bg-amber-500/10 text-amber-400"
                    : "border-linestrong bg-raise/60 text-ink4"
                }`}
              >
                <span
                  className={`h-2 w-2 rounded-full ${
                    isRunning
                      ? "bg-emerald-400"
                      : isTransitioning
                      ? "bg-amber-400 animate-pulse"
                      : "bg-ink5"
                  }`}
                />
                {isRunning
                  ? t.dashboard.active
                  : isStarting
                  ? t.dashboard.connecting
                  : isStopping
                  ? t.dashboard.stopping
                  : t.dashboard.stopped}
              </span>

              <h1
                className={`text-3xl md:text-[2.6rem] font-extrabold tracking-tight leading-none ${
                  isRunning
                    ? "bg-gradient-to-r from-emerald-300 via-emerald-400 to-teal-300 bg-clip-text text-transparent"
                    : "text-ink"
                }`}
              >
                {isRunning ? t.dashboard.bypassOnHeadline : t.dashboard.bypassOffHeadline}
              </h1>
              <p className="text-sm text-ink3 leading-relaxed">
                {isRunning ? (
                  <>
                    {t.dashboard.activePreset}{" "}
                    <span className="font-semibold text-ink">
                      {activeStrat ? activeStrat.name : status.active_strategy || t.dashboard.defaultStrategy}
                    </span>
                    .{" "}{t.dashboard.filteringNote}
                  </>
                ) : (
                  t.dashboard.hint
                )}
              </p>
              {isRunning && (status.uptime_secs > 0 || status.pid) && (
                <div className="flex items-center gap-3 text-xs font-mono text-ink4">
                  {status.uptime_secs > 0 && (
                    <span>{t.dashboard.uptime} {formatUptime(status.uptime_secs)}</span>
                  )}
                  {status.pid && <span className="text-ink5">{t.status.pid}: {status.pid}</span>}
                </div>
              )}
            </div>

            {/* Hero Power Orb */}
            <div className="flex items-center justify-center md:pr-8 shrink-0">
              <HeroOrb
                state={orbState}
                onToggle={isRunning ? onStop : onStart}
                disabled={isTransitioning}
                label={orbLabel}
              />
            </div>
          </div>

          {/* Embedded Split-Tunneling Bar */}
          <div className="mt-5 pt-4 border-t border-line">
            <SplitTunnelBar
              currentMode={splitTunnelMode}
              onModeChange={onModeChange}
              isRunning={isRunning}
              disabled={isTransitioning}
            />
          </div>
        </motion.div>

        {/* Live Service Diagnostics Quick Matrix */}
        <motion.div
          variants={{
            hidden: { opacity: 0, y: 18 },
            show: {
              opacity: 1,
              y: 0,
              transition: { duration: 0.5, ease: [0.22, 1, 0.36, 1] },
            },
          }}
          className="grid grid-cols-1 md:grid-cols-2 gap-4"
        >
          {/* YouTube Probe Card */}
          <div className="t-card p-4 flex items-center justify-between hover-lift">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-red-500/10 border border-red-500/20 text-red-400">
                <Tv className="h-5 w-5" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="text-xs font-semibold text-ink">YouTube & GoogleVideo</h4>
                  <span
                    className={`h-2 w-2 rounded-full ${
                      diagnostics.youtube.status === "operational"
                        ? "bg-emerald-400"
                        : diagnostics.youtube.status === "checking"
                        ? "bg-amber-400 animate-pulse"
                        : "bg-rose-400"
                    }`}
                  />
                </div>
                <p className="text-[11px] text-ink4 mt-0.5">
                  {diagnostics.youtube.message || t.dashboard.checkingConnection}
                </p>
              </div>
            </div>
            <div className="text-right flex flex-col items-end gap-1">
              {diagnostics.youtube.latency_ms !== null ? (
                <span className="text-xs font-mono font-semibold text-emerald-400">
                  {diagnostics.youtube.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-ink5">—</span>
              )}
              <Sparkline values={ytHist} width={110} height={30} />
            </div>
          </div>

          {/* Discord Voice & Gateway Probe Card */}
          <div className="t-card p-4 flex items-center justify-between hover-lift">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-accent-wash border border-accent-line text-accent">
                <Mic className="h-5 w-5" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="text-xs font-semibold text-ink">Discord Voice & Gateway</h4>
                  <span
                    className={`h-2 w-2 rounded-full ${
                      diagnostics.discord.status === "operational"
                        ? "bg-emerald-400"
                        : diagnostics.discord.status === "checking"
                        ? "bg-amber-400 animate-pulse"
                        : "bg-rose-400"
                    }`}
                  />
                </div>
                <p className="text-[11px] text-ink4 mt-0.5">
                  {diagnostics.discord.message || t.dashboard.checkingConnection}
                </p>
              </div>
            </div>
            <div className="text-right flex flex-col items-end gap-1">
              {diagnostics.discord.latency_ms !== null ? (
                <span className="text-xs font-mono font-semibold text-emerald-400">
                  {diagnostics.discord.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-ink5">—</span>
              )}
              <Sparkline values={dcHist} width={110} height={30} />
            </div>
          </div>
        </motion.div>

        {/* Quick Hub Navigation Cards */}
        <motion.div
          variants={{
            hidden: { opacity: 0, y: 18 },
            show: {
              opacity: 1,
              y: 0,
              transition: { duration: 0.5, ease: [0.22, 1, 0.36, 1] },
            },
          }}
          className="grid grid-cols-1 md:grid-cols-3 gap-3"
        >
          {/* Card 1: Strategies */}
          <div
            role="button"
            tabIndex={0}
            onClick={() => onNavigateTab("strategies")}
            onKeyDown={(e) => e.key === "Enter" && onNavigateTab("strategies")}
            className="group cursor-pointer t-card p-4 hover-lift"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-accent">
                <Zap className="h-4 w-4" />
                <span className="text-xs font-semibold">{t.dashboard.hubPresets}</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-accent group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              {t.dashboard.hubPresetsDesc}
            </p>
          </div>

          {/* Card 2: DNS & Speedtest */}
          <div
            role="button"
            tabIndex={0}
            onClick={() => onNavigateTab("dns")}
            onKeyDown={(e) => e.key === "Enter" && onNavigateTab("dns")}
            className="group cursor-pointer t-card p-4 hover-lift"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-accent">
                <Globe2 className="h-4 w-4" />
                <span className="text-xs font-semibold">{t.dashboard.hubDns}</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-accent group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              {t.dashboard.hubDnsDesc}
            </p>
          </div>

          {/* Card 3: Auto-Doctor & Diagnostics */}
          <div
            role="button"
            tabIndex={0}
            onClick={() => onNavigateTab("diagnostics")}
            onKeyDown={(e) => e.key === "Enter" && onNavigateTab("diagnostics")}
            className="group cursor-pointer t-card p-4 hover-lift"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-amber-400">
                <Wrench className="h-4 w-4" />
                <span className="text-xs font-semibold">{t.dashboard.hubDoctor}</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-amber-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              {t.dashboard.hubDoctorDesc}
            </p>
          </div>
        </motion.div>
      </motion.div>
    );
  }
);
