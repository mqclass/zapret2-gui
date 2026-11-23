import React from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2 } from "lucide-react";
import { SplitTunnelMode } from "../types";
import { useI18n } from "../i18n";
import { SplitTunnelBar } from "./SplitTunnelBar";
import { isAutoTuneModalOpen, isProbeCycleRunning } from "../utils/watchdog";

interface TunnelModeSectionProps {
  currentMode: SplitTunnelMode;
  /** Mode the running engine was launched with; null when the mode
   * does not apply to the active strategy (Ultimate, custom args)
   * or the backend did not report it. */
  appliedMode?: SplitTunnelMode | null;
  isRunning: boolean;
  activeStrategyId: string | null;
  activeCustomArgs?: boolean;
  onModeChange: (mode: SplitTunnelMode, opts?: { silent?: boolean }) => Promise<void>;
  disabled?: boolean;
}

/**
 * Lua tunnel modes (FULL / STRICT / GAME) with the restart-confirm flow.
 * Extracted verbatim from the expert dashboard (stage 3): the mode is
 * baked into the engine arguments, so applying a different mode to a
 * running engine means a restart. The confirmation (the app-split
 * dialog pattern) is offered only when a restart would actually change
 * something: the engine runs, the mode applies to the active strategy
 * (the backend reports an applied mode), the picked mode differs from
 * the applied one, the launch is not a one-off custom-args run (a
 * restart would silently drop those arguments), and no probe cycle
 * owns the engine. Every other case saves only.
 */
export const TunnelModeSection: React.FC<TunnelModeSectionProps> = ({
  currentMode,
  appliedMode = null,
  isRunning,
  activeStrategyId,
  activeCustomArgs = false,
  onModeChange,
  disabled = false,
}) => {
  const { t } = useI18n();

  const [pendingMode, setPendingMode] = React.useState<SplitTunnelMode | null>(null);
  const [isApplyingMode, setIsApplyingMode] = React.useState(false);
  const [modeError, setModeError] = React.useState<string | null>(null);

  const handleModeCardClick = (mode: SplitTunnelMode) => {
    setModeError(null);
    if (mode === currentMode) return;
    const restartWouldApply =
      isRunning &&
      appliedMode !== null &&
      appliedMode !== mode &&
      !activeCustomArgs &&
      !isProbeCycleRunning() &&
      !isAutoTuneModalOpen();
    if (restartWouldApply) {
      setPendingMode(mode);
    } else {
      void onModeChange(mode);
    }
  };

  const handleModeRestart = async () => {
    if (!pendingMode) return;
    const mode = pendingMode;
    setIsApplyingMode(true);
    setModeError(null);
    try {
      // Save first (silently: the "restart to apply" alert would be
      // stale the moment the restart below lands), then restart
      // through the same stop/start pair the app-split modal uses,
      // with the active strategy, not the merely selected one.
      await onModeChange(mode, { silent: true });
      await invoke("stop_supervisor");
      await invoke("start_supervisor", {
        strategyId: activeStrategyId ?? null,
        customArgs: null,
      });
      setPendingMode(null);
    } catch (err: any) {
      setModeError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsApplyingMode(false);
    }
  };

  const handleModeSaveOnly = () => {
    if (!pendingMode) return;
    const mode = pendingMode;
    setPendingMode(null);
    void onModeChange(mode);
  };

  return (
    <div className="space-y-3">
      <SplitTunnelBar
        currentMode={currentMode}
        onModeChange={handleModeCardClick}
        isRunning={isRunning}
        appliedMode={appliedMode}
        disabled={disabled || isProbeCycleRunning() || isAutoTuneModalOpen()}
      />
      {pendingMode && (
        <div className="rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-3">
          <p className="text-xs font-semibold text-amber-300">
            {t.appSplit.confirmRestartTitle}
          </p>
          <p className="mt-1 text-[11px] leading-relaxed text-amber-300/90">
            {t.appSplit.restartWarning}
          </p>
          <div className="mt-3 flex gap-2">
            <button
              onClick={handleModeRestart}
              disabled={isApplyingMode}
              className="flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold bg-accent-fill text-accent-contrast hover:opacity-90 transition-all cursor-pointer disabled:opacity-50"
            >
              {isApplyingMode && (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              )}
              <span>{t.appSplit.restartNow}</span>
            </button>
            <button
              onClick={handleModeSaveOnly}
              disabled={isApplyingMode}
              className="rounded-control px-3 py-1.5 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer disabled:opacity-50"
            >
              {t.appSplit.saveOnly}
            </button>
            <button
              onClick={() => setPendingMode(null)}
              disabled={isApplyingMode}
              className="rounded-control px-3 py-1.5 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer disabled:opacity-50"
            >
              {t.appSplit.cancel}
            </button>
          </div>
          {modeError && (
            <p className="mt-2 text-[11px] text-rose-400">{modeError}</p>
          )}
        </div>
      )}
    </div>
  );
};
