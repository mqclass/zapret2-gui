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
      if (hrs > 0) return `${hrs}ч ${mins}м ${s}с`;
      if (mins > 0) return `${mins}м ${s}с`;
      return `${s}с`;
    };

    return (
      <div className="space-y-4 pb-4">
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
                    ? "Обход Активен"
                    : isStarting
                    ? "Подключение..."
                    : isStopping
                    ? "Остановка..."
                    : "Обход Остановлен"}
                </span>

                {isRunning && status.uptime_secs > 0 && (
                  <span className="text-xs font-mono text-ink4">
                    Время работы: {formatUptime(status.uptime_secs)}
                  </span>
                )}
                {isRunning && status.pid && (
                  <span className="text-xs font-mono text-ink5">PID: {status.pid}</span>
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
                      Активный пресет:{" "}
                      <span className="font-semibold text-ink">
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

            {/* Main Power Toggle Button */}
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={isRunning ? onStop : onStart}
                disabled={isTransitioning}
                className={`relative flex items-center justify-center gap-2.5 px-7 py-3.5 rounded-panel text-sm font-semibold tracking-wide transition-all shadow-md cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed ${
                  isRunning
                    ? "bg-danger-fill hover:bg-danger-fill-hover text-white active:scale-[0.98]"
                    : "bg-accent-fill hover:bg-accent-fill-hover text-accent-contrast active:scale-[0.98]"
                }`}
              >
                {isTransitioning ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    <span>Подождите...</span>
                  </>
                ) : isRunning ? (
                  <>
                    <Square className="h-4 w-4 fill-current" />
                    <span>ОСТАНОВИТЬ ОБХОД</span>
                  </>
                ) : (
                  <>
                    <Play className="h-4 w-4 fill-current" />
                    <span>ЗАПУСТИТЬ ОБХОД</span>
                  </>
                )}
              </button>
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
          <div className="rounded-panel border border-line bg-card p-4 flex items-center justify-between">
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
                  {diagnostics.youtube.message || "Проверка связи..."}
                </p>
              </div>
            </div>
            <div className="text-right">
              {diagnostics.youtube.latency_ms !== null ? (
                <span className="text-xs font-mono font-semibold text-emerald-400">
                  {diagnostics.youtube.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-ink5">—</span>
              )}
            </div>
          </div>

          {/* Discord Voice & Gateway Probe Card */}
          <div className="rounded-panel border border-line bg-card p-4 flex items-center justify-between">
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
                  {diagnostics.discord.message || "Проверка связи..."}
                </p>
              </div>
            </div>
            <div className="text-right">
              {diagnostics.discord.latency_ms !== null ? (
                <span className="text-xs font-mono font-semibold text-emerald-400">
                  {diagnostics.discord.latency_ms} ms
                </span>
              ) : (
                <span className="text-xs font-mono text-ink5">—</span>
              )}
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
