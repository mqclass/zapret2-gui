import React from "react";
import {
  Shield,
  ShieldAlert,
  ShieldCheck,
  Play,
  Square,
  Loader2,
  Zap,
  Activity,
  Globe2,
  Tv,
  ListFilter,
  Wrench,
  Flame,
  ArrowRight,
  ExternalLink,
  ChevronRight,
} from "lucide-react";
import {
  SupervisorStatus,
  DiagnosticsReport,
  LuaStrategy,
  SplitTunnelMode,
} from "../../types";
import { useI18n } from "../../i18n";
import { SplitTunnelBar } from "../SplitTunnelBar";
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
  }) => {
    const { t } = useI18n();

    const isRunning = status.state === "running";
    const isStarting = status.state === "starting";
    const isStopping = status.state === "stopping";
    const isTransitioning = isStarting || isStopping;

    const activeStrat = strategies.find((s) => s.id === status.active_strategy);
    const selectedStrat = strategies.find((s) => s.id === selectedStrategyId);

    const formatUptime = (secs: number) => {
      const hrs = Math.floor(secs / 3600);
      const mins = Math.floor((secs % 3600) / 60);
      const s = secs % 60;
      if (hrs > 0) return `${hrs}h ${mins}m ${s}s`;
      if (mins > 0) return `${mins}m ${s}s`;
      return `${s}s`;
    };

    return (
      <div className="space-y-6 pb-6">
        {/* Hero Control Card */}
        <div className="relative overflow-hidden rounded-2xl border border-white/[0.08] bg-gradient-to-b from-slate-900/90 to-slate-950/90 p-6 shadow-2xl backdrop-blur-xl">
          {/* Background Ambient Glow */}
          <div
            className={`pointer-events-none absolute -right-16 -top-16 h-64 w-64 rounded-full blur-3xl opacity-20 transition-all duration-700 ${
              isRunning ? "bg-emerald-500" : isTransitioning ? "bg-amber-500" : "bg-blue-600"
            }`}
          />

          <div className="flex flex-col md:flex-row md:items-center justify-between gap-6 relative z-10">
            {/* Status and Active Info */}
            <div className="space-y-3">
              <div className="flex items-center gap-3">
                <span
                  className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-bold tracking-wide uppercase ${
                    isRunning
                      ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                      : isTransitioning
                      ? "border-amber-500/30 bg-amber-500/10 text-amber-400"
                      : "border-slate-700 bg-slate-800/60 text-slate-400"
                  }`}
                >
                  <span
                    className={`h-2 w-2 rounded-full ${
                      isRunning
                        ? "bg-emerald-400 shadow-[0_0_8px_#34d399]"
                        : isTransitioning
                        ? "bg-amber-400 animate-ping"
                        : "bg-slate-500"
                    }`}
                  />
                  {isRunning
                    ? "Обход Активен"
                    : isStarting
                    ? "Подключение..."
                    : isStopping
                    ? "Остановка..."
                    : "Обход Остановлен"}
                </span>

                {isRunning && status.uptime_secs > 0 && (
                  <span className="text-xs font-mono text-slate-400">
                    Аптайм: {formatUptime(status.uptime_secs)}
                  </span>
                )}
                {isRunning && status.pid && (
                  <span className="text-xs font-mono text-slate-500">PID: {status.pid}</span>
                )}
              </div>

              <div>
                <h1 className="text-2xl font-black tracking-tight text-white flex items-center gap-2.5">
                  <span>zapret2 Supervisor</span>
                  <span className="rounded-md bg-blue-500/15 border border-blue-500/30 px-2 py-0.5 text-xs font-mono font-bold text-blue-400">
                    v1.0.7
                  </span>
                </h1>
                <p className="text-xs text-slate-400 mt-1 max-w-xl leading-relaxed">
                  {isRunning ? (
                    <>
                      Активный пресет:{" "}
                      <span className="font-semibold text-white">
                        {activeStrat ? activeStrat.name : status.active_strategy || "Стандартный"}
                      </span>
                      . WinWS фильтрует трафик через WFP драйвер.
                    </>
                  ) : (
                    "Выберите нужный пресет или режим туннелирования и нажмите кнопку запуска."
                  )}
                </p>
              </div>
            </div>

            {/* Big Main Power Toggle Button */}
            <div className="flex items-center gap-3">
              <button
                onClick={isRunning ? onStop : onStart}
                disabled={isTransitioning}
                className={`relative flex items-center justify-center gap-3 px-8 py-4 rounded-xl text-sm font-bold tracking-wide transition-all shadow-xl cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed ${
                  isRunning
                    ? "bg-rose-600 hover:bg-rose-500 text-white shadow-rose-600/30 active:scale-95"
                    : "bg-gradient-to-r from-blue-600 via-indigo-600 to-cyan-500 hover:from-blue-500 hover:to-cyan-400 text-white shadow-blue-600/30 active:scale-95"
                }`}
              >
                {isTransitioning ? (
                  <>
                    <Loader2 className="h-5 w-5 animate-spin" />
                    <span>Подождите...</span>
                  </>
                ) : isRunning ? (
                  <>
                    <Square className="h-5 w-5 fill-current" />
                    <span>ОСТАНОВИТЬ ОБХОД</span>
                  </>
                ) : (
                  <>
                    <Play className="h-5 w-5 fill-current" />
                    <span>ЗАПУСТИТЬ ОБХОД</span>
                  </>
                )}
              </button>
            </div>
          </div>

          {/* Embedded Split-Tunneling and Game Mode Bar */}
          <div className="mt-6 pt-5 border-t border-white/[0.06]">
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
          <div className="rounded-xl border border-white/[0.06] bg-slate-900/60 p-4 backdrop-blur-md flex items-center justify-between">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-rose-500/10 border border-rose-500/20 text-rose-400">
                <Tv className="h-5 w-5" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="text-xs font-bold text-white">YouTube & GoogleVideo</h4>
                  <span
                    className={`h-2 w-2 rounded-full ${
                      diagnostics.youtube.status === "operational"
                        ? "bg-emerald-400 shadow-[0_0_8px_#34d399]"
                        : diagnostics.youtube.status === "checking"
                        ? "bg-amber-400 animate-pulse"
                        : "bg-rose-400"
                    }`}
                  />
                </div>
                <p className="text-[11px] text-slate-400 mt-0.5">
                  {diagnostics.youtube.message || "Проверка связи..."}
                </p>
              </div>
            </div>
            <div className="text-right">
              {diagnostics.youtube.latency_ms !== null ? (
                <span className="text-xs font-mono font-bold text-emerald-400">
                  {diagnostics.youtube.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-slate-500">—</span>
              )}
            </div>
          </div>

          {/* Discord Voice & Gateway Probe Card */}
          <div className="rounded-xl border border-white/[0.06] bg-slate-900/60 p-4 backdrop-blur-md flex items-center justify-between">
            <div className="flex items-center gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-indigo-500/10 border border-indigo-500/20 text-indigo-400">
                <Flame className="h-5 w-5" />
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <h4 className="text-xs font-bold text-white">Discord Voice & Media</h4>
                  <span
                    className={`h-2 w-2 rounded-full ${
                      diagnostics.discord.status === "operational"
                        ? "bg-emerald-400 shadow-[0_0_8px_#34d399]"
                        : diagnostics.discord.status === "checking"
                        ? "bg-amber-400 animate-pulse"
                        : "bg-rose-400"
                    }`}
                  />
                </div>
                <p className="text-[11px] text-slate-400 mt-0.5">
                  {diagnostics.discord.message || "Проверка связи..."}
                </p>
              </div>
            </div>
            <div className="text-right">
              {diagnostics.discord.latency_ms !== null ? (
                <span className="text-xs font-mono font-bold text-emerald-400">
                  {diagnostics.discord.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-slate-500">—</span>
              )}
            </div>
          </div>
        </div>

        {/* Quick Hub Navigation Cards */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {/* Card 1: Strategies */}
          <div
            onClick={() => onNavigateTab("strategies")}
            className="group cursor-pointer rounded-xl border border-white/[0.06] bg-slate-900/40 p-4 hover:border-blue-500/40 hover:bg-slate-900/80 transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-blue-400">
                <Zap className="h-4 w-4" />
                <span className="text-xs font-bold">Пресеты обхода</span>
              </div>
              <ChevronRight className="h-4 w-4 text-slate-500 group-hover:text-blue-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-slate-400 leading-relaxed">
              Выбор проверенных конфигураций: Flowseal Alt, Fake TLS Auto, или создание своего пресета.
            </p>
          </div>

          {/* Card 2: DNS & 4K Speedtest */}
          <div
            onClick={() => onNavigateTab("dns")}
            className="group cursor-pointer rounded-xl border border-white/[0.06] bg-slate-900/40 p-4 hover:border-cyan-500/40 hover:bg-slate-900/80 transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-cyan-400">
                <Globe2 className="h-4 w-4" />
                <span className="text-xs font-bold">DNS & Тест скорости 4K</span>
              </div>
              <ChevronRight className="h-4 w-4 text-slate-500 group-hover:text-cyan-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-slate-400 leading-relaxed">
              1-клик переключение на Cloudflare 1.1.1.1 / Google DNS и замер реальной скорости YouTube.
            </p>
          </div>

          {/* Card 3: Auto-Doctor & Diagnostics */}
          <div
            onClick={() => onNavigateTab("diagnostics")}
            className="group cursor-pointer rounded-xl border border-white/[0.06] bg-slate-900/40 p-4 hover:border-amber-500/40 hover:bg-slate-900/80 transition-all"
          >
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2 text-amber-400">
                <Wrench className="h-4 w-4" />
                <span className="text-xs font-bold">Авто-Доктор Сети</span>
              </div>
              <ChevronRight className="h-4 w-4 text-slate-500 group-hover:text-amber-400 group-hover:translate-x-0.5 transition-all" />
            </div>
            <p className="text-[11px] text-slate-400 leading-relaxed">
              Сброс Winsock, очистка DNS кэша, восстановление WinDivert и автоподбор пресета под провайдера.
            </p>
          </div>
        </div>
      </div>
    );
  }
);
