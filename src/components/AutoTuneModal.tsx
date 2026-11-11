import React, { useState, useCallback, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import { Sparkles, Zap, Loader2, X, HelpCircle } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import {
  StrategyProbeResult,
  SupervisorStatusResponse,
} from "../types";
import { ClassifierModal, ClassifierTargetRequest } from "./ClassifierModal";
import { ProbeStatusBadge } from "./ProbeStatusBadge";
import { shouldOfferClassifier } from "../utils/classifier";
import { NavTab } from "./Sidebar";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";
import {
  isProbeCycleRunning,
  setAutoTuneModalOpenFlag,
  setProbeCycleRunning,
} from "../utils/watchdog";

// Candidates probed in order. Custom user profiles are out of scope for
// the first slice: these cover the shipped engines. The three new winws2
// lua presets (yt4k, discord-light, tspu-max) are included; sites-light
// is deliberately left out: the weakest tier cannot win YouTube/Discord
// probes, it is a manual pick for weak hardware.
// Exported: the watchdog's escalation cycle reuses this exact list
// instead of maintaining a second one.
export const AUTO_TUNE_CANDIDATES = [
  "flowseal-alt",
  "flowseal-alt11",
  "flowseal-fake-tls-auto",
  "zapret2-ru-ultimate",
  "flowseal-general",
  "yt4k",
  "discord-light",
  "tspu-max",
  "extsplit",
  "firstbyte",
  "syndata",
  "ipfrag",
  // Imported kinlay0 strategies: new techniques, probed after the
  // proven candidates — they exist for providers where the whole
  // standard pool fails.
  "kinlay0-double-fake",
  "kinlay0-double-fake-alt",
  "kinlay0-double-fake-alt2",
  "kinlay0-single-fake",
  "kinlay0-single-fake-alt",
  "kinlay0-hostfakesplit-ts",
  "kinlay0-hostfakesplit-ts-alt",
];

export interface UseAutoTuneOptions {
  onSelectAndStartStrategy?: (strategyId: string) => void;
  onNavigateTab?: (tab: NavTab) => void;
  /** Sink for user-initiated action errors (auto-tune pre-check failures). */
  onError?: (message: string | null) => void;
}

export interface UseAutoTuneResult {
  isAutoTuning: boolean;
  startAutoTune: () => void;
  /** The results modal (portal) plus the classifier modal it can offer. */
  autoTuneModal: React.ReactNode;
}

/**
 * Auto-tune: real per-strategy probing. The engine restarts on each
 * candidate, so connections drop during the run and the original state
 * is restored at the end (even on cancel or error). Extracted from
 * DiagnosticsCard so the dashboard's «Подобрать автоматически» button
 * opens the exact same modal with the exact same cycle; each host
 * component owns an instance, and the shared probe lock in
 * utils/watchdog keeps two cycles from ever interleaving.
 */
export function useAutoTune({
  onSelectAndStartStrategy,
  onNavigateTab,
  onError,
}: UseAutoTuneOptions): UseAutoTuneResult {
  const { t } = useI18n();
  const reportError = useCallback(
    (message: string | null) => {
      onError?.(message);
    },
    [onError]
  );

  const [isAutoTuning, setIsAutoTuning] = useState(false);
  const [autoTuneProgress, setAutoTuneProgress] = useState<{ index: number; total: number; id: string } | null>(null);
  const [probeResults, setProbeResults] = useState<StrategyProbeResult[] | null>(null);
  const [isAutoTuneModalOpen, setIsAutoTuneModalOpen] = useState(false);
  const autoTuneCancelRef = useRef(false);
  const autoTuneRunningRef = useRef(false);

  // Block classifier hint (§5): offered after an honest auto-tune
  // failure, behind the §5.2 gate in utils/classifier.
  const [classifierReq, setClassifierReq] = useState<ClassifierTargetRequest | null>(null);
  const [isClassifierOpen, setIsClassifierOpen] = useState(false);
  const [classifierBusy, setClassifierBusy] = useState(false);

  const handleOfferClassifier = useCallback(() => {
    // §4.2: another cycle (watchdog) may have claimed the probe lock
    // since the results rendered — never start on top of it.
    if (isProbeCycleRunning()) {
      setClassifierBusy(true);
      return;
    }
    setClassifierBusy(false);
    setClassifierReq({ target: "youtube", targetName: "YouTube" });
    setIsClassifierOpen(true);
  }, []);

  // Publish the modal state for the watchdog: it never ticks while the
  // manual modal is open, and never starts a cycle under it. The
  // cleanup matters: the host unmounts when the user leaves the tab,
  // and a leaked "open" flag would silence the watchdog indefinitely.
  useEffect(() => {
    setAutoTuneModalOpenFlag(isAutoTuneModalOpen);
    return () => setAutoTuneModalOpenFlag(false);
  }, [isAutoTuneModalOpen]);

  const handleAutoTune = useCallback(async () => {
    // Ref guard, set synchronously: the state guard alone is stale until
    // re-render, so a double click before then would start two loops
    // whose probes interleave and get attributed to the wrong strategy.
    if (autoTuneRunningRef.current) return;
    // The watchdog escalation cycle owns the same probe lock: a probe
    // restarts the engine, so two loops must never interleave. Claimed
    // synchronously here, cleared in the pre-check aborts and the loop
    // finally below.
    if (isProbeCycleRunning()) {
      reportError(t.diagnostics.autoTuneBusy);
      return;
    }
    setProbeCycleRunning(true);
    autoTuneRunningRef.current = true;
    autoTuneCancelRef.current = false;
    reportError(null);

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
        reportError(t.diagnostics.autoTuneBusy);
        autoTuneRunningRef.current = false;
        setProbeCycleRunning(false);
        return;
      }
      wasRunning = st.supervisor.state === "running";
      originalId = st.supervisor.active_strategy ?? null;
    } catch (err) {
      console.error("Auto-tune pre-check error:", err);
      reportError(formatErrorMessage(err));
      autoTuneRunningRef.current = false;
      setProbeCycleRunning(false);
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
          if (res == null || typeof res !== "object") {
            // A transport that resolves without a value must not poison
            // the results array: a null entry crashes the sorted results
            // comparator (null.score) and the ErrorBoundary kills the
            // dashboard. Treat it exactly like a failed probe.
            throw new Error("Проверка не вернула результат");
          }
          results.push(res);
          setProbeResults([...results]);
          if (res.error && res.error.includes("ADMIN_REQUIRED")) {
            reportError(t.adminGate.message);
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
          reportError(formatErrorMessage(err));
        }
      }
      setIsAutoTuning(false);
      setAutoTuneProgress(null);
      autoTuneRunningRef.current = false;
      setProbeCycleRunning(false);
    }
  }, [onSelectAndStartStrategy, t, reportError]);

  const handleCancelAutoTune = useCallback(() => {
    autoTuneCancelRef.current = true;
    setIsAutoTuneModalOpen(false);
  }, []);

  const sortedProbeResults = probeResults
    ? [...probeResults]
        .filter((r): r is StrategyProbeResult => r != null && typeof r === "object")
        .sort(
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

  const autoTuneModal = (
    <>
      {/* Auto-Tune Results Modal (portal: see CustomProfileModal — glass
          ancestors would otherwise confine the fixed backdrop) */}
      {isAutoTuneModalOpen && (isAutoTuning || probeResults) &&
        createPortal(
          <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 overflow-y-auto no-scrollbar modal-backdrop-enter">
            <div className="m-auto w-full max-w-lg rounded-card border border-line bg-card p-5 shadow-2xl modal-panel-enter">
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

                {/* Classifier hint (§5): only behind the §5.2 gate —
                    at least one candidate was really started and
                    measured, and still nothing scored. */}
                {shouldOfferClassifier(probeResults, autoTuneWinner) && (
                  <div className="mt-3 rounded-panel border border-raise bg-cardin/40 p-3">
                    <p className="text-xs text-ink2">{t.classifier.hintText}</p>
                    {classifierBusy && (
                      <p className="mt-1.5 text-[11px] text-amber-300">{t.classifier.busyNote}</p>
                    )}
                    <button
                      type="button"
                      onClick={handleOfferClassifier}
                      className="mt-2 flex items-center gap-1 rounded-control bg-accent px-3 py-1.5 text-[11px] font-semibold text-white hover:opacity-90 cursor-pointer transition-opacity"
                    >
                      <HelpCircle className="h-3 w-3" />
                      <span>{t.classifier.hintBtn}</span>
                    </button>
                  </div>
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
                            YouTube <ProbeStatusBadge status={res.youtube_status} />
                          </span>
                          <span className="flex items-center gap-1 text-ink4">
                            Discord <ProbeStatusBadge status={res.discord_status} />
                          </span>
                          <span className="flex items-center gap-1 text-ink4">
                            Voice <ProbeStatusBadge status={res.voice_status} />
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
          </div>,
          document.body
        )}

      {/* Block Classifier Modal (from the §5 hint) */}
      <ClassifierModal
        open={isClassifierOpen}
        request={classifierReq}
        onClose={() => setIsClassifierOpen(false)}
        onOpenStrategies={() => onNavigateTab?.("presets")}
        onRerunAutoTune={() => {
          setIsAutoTuneModalOpen(false);
          handleAutoTune();
        }}
      />
    </>
  );

  return { isAutoTuning, startAutoTune: handleAutoTune, autoTuneModal };
}
