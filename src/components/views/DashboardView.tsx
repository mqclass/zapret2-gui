import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion, Variants } from "framer-motion";
import {
  Zap,
  Globe2,
  Tv,
  Wrench,
  Mic,
  ChevronRight,
  AlertTriangle,
  Loader2,
  Activity,
  RefreshCw,
  Sparkles,
  Eye,
  AppWindow,
  MessageCircle,
  Volume2,
  Send,
} from "lucide-react";
import {
  SupervisorStatus,
  DiagnosticsReport,
  LuaStrategy,
  SplitTunnelMode,
  UpdateInfo,
  ProbeMetric,
} from "../../types";
import { useI18n } from "../../i18n";
import { HeroOrb, OrbState } from "../HeroOrb";
import { Sparkline } from "../Sparkline";
import { UpdateBanner } from "../UpdateBanner";
import { TransportsSection } from "../TransportsSection";
import { GoalPicker } from "../GoalPicker";
import { useAutoTune } from "../AutoTuneModal";
import { AppSplitModal } from "../AppSplitModal";
import { CustomProfileModal } from "../CustomProfileModal";
import { NavTab } from "../Sidebar";

interface DashboardViewProps {
  /** Shell view mode (redesign): false = Простой вид, true = Эксперт. */
  expertMode?: boolean;
  status: SupervisorStatus;
  diagnostics: DiagnosticsReport;
  strategies: LuaStrategy[];
  selectedStrategyId: string;
  splitTunnelMode: SplitTunnelMode;
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
  /** Strategy watchdog state, for the simple view's status-only row. */
  watchdogOn: boolean;
  watchdogIntervalMinutes: number;
  /** Wizard «Свой конфиг» goal: saves through the same App handler as the presets tab. */
  onSaveCustomProfile: (strategy: LuaStrategy) => Promise<void>;
}

const listVariants: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.08, delayChildren: 0.05 } },
};
const itemVariants: Variants = {
  hidden: { opacity: 0, y: 18 },
  show: {
    opacity: 1,
    y: 0,
    transition: { duration: 0.5, ease: [0.22, 1, 0.36, 1] },
  },
};

export const DashboardView: React.FC<DashboardViewProps> = React.memo(
  ({
    expertMode = false,
    status,
    diagnostics,
    strategies,
    selectedStrategyId,
    splitTunnelMode,
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
    watchdogOn,
    watchdogIntervalMinutes,
    onSaveCustomProfile,
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

    // Tunnel mode status (redesign stage 3): the controls moved to
    // the presets tab — here the dashboard only shows which mode is
    // active, and whether the running engine still uses another one.
    const appliedMode = (status.split_tunnel_applied_mode ??
      null) as SplitTunnelMode | null;

    const formatUptime = (secs: number) => {
      const hrs = Math.floor(secs / 3600);
      const mins = Math.floor((secs % 3600) / 60);
      const s = secs % 60;
      const { uptimeH: h, uptimeM: m, uptimeS: sec } = t.dashboard;
      if (hrs > 0) return `${hrs}${h} ${mins}${m} ${s}${sec}`;
      if (mins > 0) return `${mins}${m} ${s}${sec}`;
      return `${s}${sec}`;
    };

    // --- Simple view: auto-tune, wizard custom goal, app-split summary ---

    // The very same auto-tune cycle + modal the diagnostics tab hosts;
    // pre-check errors land in a small line under the strategy row.
    const [autoTuneError, setAutoTuneError] = React.useState<string | null>(null);
    const { isAutoTuning, startAutoTune, autoTuneModal } = useAutoTune({
      onSelectAndStartStrategy: onStartStrategy,
      onNavigateTab,
      onError: setAutoTuneError,
    });

    const [isCustomModalOpen, setIsCustomModalOpen] = React.useState(false);
    const [isAppSplitOpen, setIsAppSplitOpen] = React.useState(false);

    // App-split one-liner: mode + selected-app count, read from the
    // same config the modal edits; refreshed on mount and after the
    // modal closes (its Apply path lives inside the modal).
    const [appSplitSummary, setAppSplitSummary] = React.useState<{
      mode: string;
      count: number;
    } | null>(null);
    const refreshAppSplitSummary = React.useCallback(async () => {
      try {
        const cfg = await invoke<{ mode: string; apps: string[] }>(
          "get_app_split_config"
        );
        setAppSplitSummary({ mode: cfg.mode, count: cfg.apps?.length ?? 0 });
      } catch {
        // Row falls back to the "off" text; the modal itself reports errors.
      }
    }, []);
    React.useEffect(() => {
      void refreshAppSplitSummary();
    }, [refreshAppSplitSummary]);

    const statusPill = (
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
    );

    const banners = (
      <>
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
      </>
    );

    // ------------------------------------------------------------------
    // Простой вид: hero, wizard, service cards, one status card.
    // ------------------------------------------------------------------
    if (!expertMode) {
      const displayStratName = isRunning
        ? activeStrat?.name ?? status.active_strategy ?? t.dashboard.defaultStrategy
        : selectedStrat?.name ?? selectedStrategyId;

      const renderServiceCard = (
        name: string,
        metric: ProbeMetric | undefined,
        icon: React.ReactNode,
        tileClass: string
      ) => {
        const st = metric?.status ?? "checking";
        const word =
          st === "operational"
            ? t.dashboard.svcWorks
            : st === "degraded"
            ? t.dashboard.svcSlow
            : st === "blocked"
            ? t.dashboard.svcDown
            : t.dashboard.svcChecking;
        const dot =
          st === "operational"
            ? "bg-emerald-400"
            : st === "degraded"
            ? "bg-amber-400"
            : st === "blocked"
            ? "bg-rose-400"
            : "bg-amber-400 animate-pulse";
        return (
          <div className="t-card p-2.5 flex items-center gap-3">
            <div
              className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-panel border ${tileClass}`}
            >
              {icon}
            </div>
            <div className="min-w-0">
              {/* Name on its own line: «Голосовой чат» no longer loses
                  its tail to the status dot beside it. */}
              <div className="text-xs font-semibold text-ink truncate">{name}</div>
              <div className="text-[11px] text-ink4 mt-0.5 flex items-center gap-1.5">
                <span className={`h-2 w-2 shrink-0 rounded-full ${dot}`} />
                <span className="truncate">
                  {word}
                  {metric?.latency_ms != null ? ` ${metric.latency_ms} ms` : ""}
                </span>
              </div>
            </div>
          </div>
        );
      };

      const appSplitText =
        appSplitSummary === null || appSplitSummary.mode === "disabled"
          ? t.dashboard.appsOff
          : appSplitSummary.mode === "allowlist"
          ? t.dashboard.appsOnly.replace("{n}", String(appSplitSummary.count))
          : t.dashboard.appsOutside.replace("{n}", String(appSplitSummary.count));

      return (
        <motion.div
          className="space-y-2.5 pb-3"
          initial="hidden"
          animate="show"
          variants={listVariants}
        >
          {banners}

          {/* Hero: orb + plain-language status */}
          <motion.div
            variants={itemVariants}
            className="t-card relative overflow-hidden px-5 py-4"
          >
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
            <div className="relative flex items-center justify-between gap-6">
              <div className="space-y-2 max-w-xl">
                {statusPill}
                <h1
                  className={`text-3xl font-extrabold tracking-tight leading-none ${
                    isRunning
                      ? "bg-gradient-to-r from-emerald-300 via-emerald-400 to-teal-300 bg-clip-text text-transparent"
                      : "text-ink"
                  }`}
                >
                  {isRunning ? t.dashboard.bypassOnHeadline : t.dashboard.bypassOffHeadline}
                </h1>
                <p className="text-sm text-ink3 leading-relaxed">
                  {isRunning ? t.dashboard.simpleOnText : t.dashboard.simpleOffText}
                </p>
                {isRunning && status.uptime_secs > 0 && (
                  <div className="text-xs text-ink4">
                    {t.dashboard.uptime} {formatUptime(status.uptime_secs)}
                  </div>
                )}
              </div>
              <div className="flex items-center justify-center shrink-0">
                <HeroOrb
                  state={orbState}
                  onToggle={isRunning ? onStop : onStart}
                  disabled={isTransitioning}
                  label={orbLabel}
                  compact
                />
              </div>
            </div>
          </motion.div>

          {/* Wizard: what do you want to unblock? */}
          <motion.div variants={itemVariants}>
            <GoalPicker
              strategies={strategies}
              selectedStrategyId={selectedStrategyId}
              disabled={isTransitioning}
              onSelectStrategy={onSelectStrategy}
              onStartStrategy={onStartStrategy}
              onCustomGoal={() => setIsCustomModalOpen(true)}
              compact
            />
          </motion.div>

          {/* Service availability: four compact cards from the live probes */}
          <motion.div variants={itemVariants}>
            <div className="flex items-center justify-between mb-1.5 px-0.5">
              <div className="flex items-center gap-2">
                <Activity className="h-4 w-4 text-accent" />
                <h2 className="text-[13px] font-bold text-ink">
                  {t.dashboard.servicesTitle}
                </h2>
              </div>
              <button
                type="button"
                onClick={onRefreshProbes}
                disabled={isRefreshingProbes}
                className="flex items-center gap-1.5 text-[11px] px-2.5 py-1 rounded-control bg-raise/60 hover:bg-hovert border border-line text-ink3 hover:text-ink transition-all cursor-pointer disabled:opacity-50"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${isRefreshingProbes ? "animate-spin text-accent" : ""}`}
                />
                <span>{t.dashboard.checkBtn}</span>
              </button>
            </div>
            <div className="grid grid-cols-2 lg:grid-cols-4 gap-2.5">
              {renderServiceCard(
                "YouTube",
                diagnostics.youtube,
                <Tv className="h-4 w-4" />,
                "bg-red-500/10 border-red-500/20 text-red-400"
              )}
              {renderServiceCard(
                "Discord",
                diagnostics.discord,
                <MessageCircle className="h-4 w-4" />,
                "bg-indigo-500/10 border-indigo-500/20 text-indigo-300"
              )}
              {renderServiceCard(
                t.dashboard.svcVoice,
                diagnostics.discord_voice,
                <Volume2 className="h-4 w-4" />,
                "bg-violet-500/10 border-violet-500/20 text-violet-300"
              )}
              {renderServiceCard(
                "Telegram",
                diagnostics.telegram,
                <Send className="h-4 w-4" />,
                "bg-sky-500/10 border-sky-500/20 text-sky-300"
              )}
            </div>
          </motion.div>

          {/* One status card: strategy row, then watchdog + per-app
              routing side by side (one row instead of two — the simple
              view must fit 1050×760 without scrolling). */}
          <motion.div variants={itemVariants} className="t-card p-3.5 space-y-2.5">
            <div className="flex flex-wrap items-center gap-3">
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-accent-wash border border-accent-line text-accent">
                <Zap className="h-4 w-4" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[11px] text-ink4">{t.dashboard.strategyLabel}</div>
                <div className="text-sm font-semibold text-ink truncate">
                  {displayStratName}
                </div>
              </div>
              <button
                type="button"
                onClick={startAutoTune}
                disabled={isAutoTuning || isTransitioning}
                className="flex items-center gap-1.5 rounded-control border border-accent-line bg-accent-wash px-3 py-1 text-xs font-semibold text-accent-soft hover:bg-accent-fill/20 hover:text-ink transition-all cursor-pointer disabled:opacity-50"
              >
                {isAutoTuning ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Sparkles className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {isAutoTuning ? t.diagnostics.autoTuneRunning : t.dashboard.autoTuneBtn}
                </span>
              </button>
              <button
                type="button"
                onClick={() => onNavigateTab("presets")}
                className="rounded-control px-3 py-1 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer"
              >
                {t.dashboard.changeStrategy}
              </button>
            </div>
            {autoTuneError && (
              <p className="text-[11px] text-rose-400">{autoTuneError}</p>
            )}

            <div className="border-t border-line" />

            <div className="grid grid-cols-2 divide-x divide-line">
              <div className="flex items-center gap-2.5 pr-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-panel bg-raise/60 border border-line text-ink3">
                  <Eye className="h-4 w-4" />
                </div>
                <p className="text-xs text-ink2 leading-snug">
                  {watchdogOn
                    ? `${t.dashboard.watchdogOnText} · ${t.dashboard.watchdogEvery.replace(
                        "{n}",
                        String(watchdogIntervalMinutes)
                      )}`
                    : t.dashboard.watchdogOffText}
                </p>
              </div>
              <div className="flex items-center gap-2.5 pl-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-panel bg-raise/60 border border-line text-ink3">
                  <AppWindow className="h-4 w-4" />
                </div>
                <p className="flex-1 min-w-0 text-xs text-ink2 leading-snug">{appSplitText}</p>
                <button
                  type="button"
                  onClick={() => setIsAppSplitOpen(true)}
                  className="rounded-control px-2.5 py-1 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer shrink-0"
                >
                  {t.appSplit.configure}
                </button>
              </div>
            </div>
          </motion.div>

          {autoTuneModal}
          <AppSplitModal
            isOpen={isAppSplitOpen}
            onClose={() => {
              setIsAppSplitOpen(false);
              void refreshAppSplitSummary();
            }}
          />
          <CustomProfileModal
            isOpen={isCustomModalOpen}
            onClose={() => setIsCustomModalOpen(false)}
            onSave={onSaveCustomProfile}
            editingStrategy={null}
          />
        </motion.div>
      );
    }

    // ------------------------------------------------------------------
    // Эксперт: the full pre-redesign dashboard, nothing removed —
    // hero with the tunnel-mode bar, raw probe matrix, transports,
    // quick hub links.
    // ------------------------------------------------------------------
    return (
      <motion.div
        className="space-y-4 pb-4"
        initial="hidden"
        animate="show"
        variants={listVariants}
      >
        {banners}
        {/* Hero Control Card */}
        <motion.div
          variants={itemVariants}
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
              {statusPill}

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

          {/* Tunnel mode: status only — the controls live on the
              presets tab now (redesign stage 3). */}
          <div className="mt-5 pt-4 border-t border-line flex items-center justify-between gap-3">
            <div className="text-xs text-ink3 min-w-0">
              <span className="font-semibold text-ink">
                {t.presetsTab.tunnelModeLine}:{" "}
              </span>
              {splitTunnelMode === "game_mode"
                ? t.sidebar.modeGame
                : splitTunnelMode === "strict"
                ? t.sidebar.modeStrict
                : t.sidebar.modeFull}
              {appliedMode !== null && appliedMode !== splitTunnelMode && (
                <span className="text-amber-400/90">
                  {" "}
                  · {t.splitTunnel.restartHint}
                </span>
              )}
            </div>
            <button
              type="button"
              onClick={() => onNavigateTab("presets")}
              className="shrink-0 rounded-control border border-line bg-raise hover:bg-hovert px-2.5 py-1.5 text-xs font-medium text-ink3 transition-colors cursor-pointer"
            >
              {t.presetsTab.changeMode}
            </button>
          </div>
        </motion.div>

        {/* Live Service Diagnostics Quick Matrix */}
        <motion.div
          variants={itemVariants}
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

        {/* Camouflage transports (self-contained section) */}
        <motion.div variants={itemVariants}>
          <TransportsSection />
        </motion.div>

        {/* Quick Hub Navigation Cards */}
        <motion.div
          variants={itemVariants}
          className="grid grid-cols-1 md:grid-cols-3 gap-3"
        >
          {/* Card 1: Strategies */}
          <div
            role="button"
            tabIndex={0}
            onClick={() => onNavigateTab("presets")}
            onKeyDown={(e) => e.key === "Enter" && onNavigateTab("presets")}
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
