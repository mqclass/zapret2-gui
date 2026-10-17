import React from "react";
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
    const pingHist = React.useRef<Record<string, (number | null)[]>>({});
    const pushPing = (key: string, v: number | null) => {
      const h = pingHist.current[key] ?? [];
      h.push(v);
      if (h.length > 40) h.shift();
      pingHist.current[key] = h;
      return h;
    };
    const ytHist = pushPing("yt", diagnostics.youtube.latency_ms);
    const dcHist = pushPing("dc", diagnostics.discord.latency_ms);

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
      if (hrs > 0) return `${hrs}ч ${mins}м ${s}с`;
      if (mins > 0) return `${mins}м ${s}с`;
      return `${s}с`;
    };

    return (
      <div className="space-y-4 pb-4">
        {/* Update availability banner (announces only — no silent downloads) */}
        {updateInfo?.update_available && (
          <UpdateBanner
            info={updateInfo}
            onOpenDialog={onOpenUpdateDialog}
            onDismiss={onDismissUpdate}
          />
        )}
        {/* Hero Control Card */}
        <div className="rounded-card border border-line bg-card p-6 shadow-sm">
          <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
            {/* Status and Active Info */}
            <div className="space-y-3">
              <div className="flex items-center gap-3">
                <span
                  className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-semibold tracking-wide uppercase ${
                    isRunning
                      ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
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

                {isRunning && status.uptime_secs > 0 && (
                  <span className="text-xs font-mono text-ink4">
                    {t.dashboard.uptime} {formatUptime(status.uptime_secs)}
                  </span>
                )}
                {isRunning && status.pid && (
                  <span className="text-xs font-mono text-ink5">{t.status.pid}: {status.pid}</span>
                )}
              </div>

              <div>
                <h1 className="text-2xl font-bold tracking-tight text-ink flex items-center gap-2.5">
                  <span>zapret2</span>
                  <span className="rounded-chip bg-accent-wash border border-accent-line px-2 py-0.5 text-xs font-mono font-semibold text-accent">
                    {t.header.version}
                  </span>
                </h1>
                <p className="text-xs text-ink4 mt-1 max-w-xl leading-relaxed">
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
              </div>
            </div>

            {/* Hero Power Orb */}
            <div className="flex items-center justify-center md:justify-end md:pr-4">
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
        </div>

        {/* Live Service Diagnostics Quick Matrix */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {/* YouTube Probe Card */}
          <div
            className="rounded-panel border border-line bg-card p-4 flex items-center justify-between hover-lift stagger-item"
            style={{ "--stagger-i": 0 } as React.CSSProperties}
          >
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
          <div
            className="rounded-panel border border-line bg-card p-4 flex items-center justify-between hover-lift stagger-item"
            style={{ "--stagger-i": 1 } as React.CSSProperties}
          >
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
        </div>

        {/* Quick Hub Navigation Cards */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
          {/* Card 1: Strategies */}
          <div
            onClick={() => onNavigateTab("strategies")}
            className="group cursor-pointer rounded-panel border border-line bg-card p-4 hover:border-linestrong hover:bg-hovert transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-accent">
                <Zap className="h-4 w-4" />
                <span className="text-xs font-semibold">Пресеты обхода</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-accent group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              Выбор готовых конфигураций: Flowseal Alt, Fake TLS Auto, или создание своего профиля.
            </p>
          </div>

          {/* Card 2: DNS & Speedtest */}
          <div
            onClick={() => onNavigateTab("dns")}
            className="group cursor-pointer rounded-panel border border-line bg-card p-4 hover:border-linestrong hover:bg-hovert transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-cyan-400">
                <Globe2 className="h-4 w-4" />
                <span className="text-xs font-semibold">DNS & Тест скорости</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-cyan-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              Переключение на Cloudflare 1.1.1.1 / Google DNS и замер реального видеопотока YouTube.
            </p>
          </div>

          {/* Card 3: Auto-Doctor & Diagnostics */}
          <div
            onClick={() => onNavigateTab("diagnostics")}
            className="group cursor-pointer rounded-panel border border-line bg-card p-4 hover:border-linestrong hover:bg-hovert transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-amber-400">
                <Wrench className="h-4 w-4" />
                <span className="text-xs font-semibold">Авто-Доктор Сети</span>
              </div>
              <ChevronRight className="h-4 w-4 text-ink5 group-hover:text-amber-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-ink4 leading-relaxed">
              Сброс Winsock, очистка DNS кэша, восстановление WinDivert и автоподбор пресета.
            </p>
          </div>
        </div>
      </div>
    );
  }
);
