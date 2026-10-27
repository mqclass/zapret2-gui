/**
 * Watchdog hook: periodic light checks of the current strategy, automatic
 * escalation on confirmed degradation. Used once, in App.
 *
 * Tick rules:
 * - Only while the engine is running. Stopped/starting/stopping ticks are
 *   skipped and do not count toward the failure streak.
 * - Never while the manual auto-tune modal is open or a probe cycle is
 *   already running (same lock the modal respects).
 * - A failed light check (refresh_probes error) is not degradation
 *   evidence: it is skipped, not counted. A mid-flight "checking"
 *   report is likewise skipped, not treated as a clean bill of health.
 * - Degradation must repeat on 2 consecutive ticks (15 minutes apart by
 *   default) before escalation runs, so one blip never restarts the engine.
 * - The failure streak resets whenever the active strategy changes, for
 *   any reason: a new strategy starts with a clean slate.
 * - Escalation ranks candidates and compares against the present on the
 *   YouTube/Discord subscore only (0-4): the voice probe is an HTTPS
 *   endpoint that reads Degraded even when real UDP voice works, so it
 *   must not decide the switch. Rollback is manual, via the Strategies tab.
 * - A cycle takes minutes, so before applying the winner the hook
 *   re-validates that nothing changed mid-cycle: still enabled, engine
 *   still running, same strategy the trigger fired on, modal still
 *   closed. A user switch, stop, or disable during the cycle wins over
 *   the watchdog. The toast fires only after verifying the engine
 *   actually moved to the winner.
 */

import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";
import { notifyUser } from "./notify";
import { AUTO_TUNE_CANDIDATES } from "../components/DiagnosticsCard";
import {
  degradedServices,
  isAutoTuneModalOpen,
  isProbeCycleRunning,
  ytDiscScore,
  probeYtDiscScore,
  pickWinner,
  runHeadlessAutoTune,
} from "./watchdog";
import type {
  DiagnosticsReport,
  LuaStrategy,
  StrategyProbeResult,
  SupervisorStatusResponse,
} from "../types";

export interface UseWatchdogOptions {
  enabled: boolean;
  intervalMinutes: number;
  engineState: string;
  activeStrategyId: string | null;
  strategies: LuaStrategy[];
  onApplyStrategy: (
    id: string,
    opts?: { silent?: boolean }
  ) => Promise<void>;
}

export function useWatchdog(options: UseWatchdogOptions): void {
  const { t } = useI18n();
  const optsRef = useRef(options);
  optsRef.current = options;
  const tRef = useRef(t);
  tRef.current = t;
  const failuresRef = useRef(0);
  const escalatingRef = useRef(false);
  const tickInFlightRef = useRef(false);
  const lastStrategyRef = useRef<string | null>(options.activeStrategyId);

  // A strategy change (manual or watchdog-driven) resets the streak.
  useEffect(() => {
    if (options.activeStrategyId !== lastStrategyRef.current) {
      lastStrategyRef.current = options.activeStrategyId;
      failuresRef.current = 0;
    }
  }, [options.activeStrategyId]);

  useEffect(() => {
    // Fresh streak on every (re)arm: disabling, enabling, or changing
    // the cadence all start from zero, so a stale streak can never
    // escalate on a single fresh tick after re-enable.
    failuresRef.current = 0;
    if (!options.enabled) return;
    const ms =
      Math.min(120, Math.max(5, options.intervalMinutes)) * 60_000;
    // Immediate first tick on mount/enable: the tick itself no-ops
    // unless the engine is running, so this cannot start an engine.
    void tick();
    const timer = window.setInterval(() => {
      void tick();
    }, ms);
    return () => window.clearInterval(timer);
    // Tick reads fresh values from optsRef; only the cadence restarts here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.enabled, options.intervalMinutes]);

  async function tick(): Promise<void> {
    // A tick is async and the interval does not await it: a slow
    // refresh_probes (slow network) could otherwise overlap the next
    // tick, and two overlapping ticks would each count the same
    // degradation, escalating after one real interval instead of two
    // consecutive ticks. The in-flight guard drops the overlap.
    if (tickInFlightRef.current) return;
    tickInFlightRef.current = true;
    try {
      await tickOnce();
    } finally {
      tickInFlightRef.current = false;
    }
  }

  async function tickOnce(): Promise<void> {
    const o = optsRef.current;
    if (!o.enabled || escalatingRef.current) return;
    if (o.engineState !== "running") return;
    if (isAutoTuneModalOpen() || isProbeCycleRunning()) return;
    let report: DiagnosticsReport;
    try {
      // Light check: plain HTTP through the running engine, no restart.
      report = await invoke<DiagnosticsReport>("refresh_probes");
    } catch (err) {
      console.error("Watchdog probe error:", err);
      return;
    }
    // State may have changed during the probe: re-check the guards.
    const o2 = optsRef.current;
    if (!o2.enabled || o2.engineState !== "running") return;
    if (isAutoTuneModalOpen() || isProbeCycleRunning()) return;
    // A mid-flight report proves nothing either way: skip, do not reset.
    if (
      report.youtube.status === "checking" ||
      report.discord.status === "checking"
    ) {
      return;
    }
    const bad = degradedServices(report);
    if (bad.length === 0) {
      failuresRef.current = 0;
      return;
    }
    failuresRef.current += 1;
    if (failuresRef.current < 2) return;
    failuresRef.current = 0;
    await escalate(report, bad, o2.activeStrategyId);
  }

  async function escalate(
    triggerReport: DiagnosticsReport,
    bad: string[],
    triggerId: string | null
  ): Promise<void> {
    if (escalatingRef.current || isProbeCycleRunning()) return;
    escalatingRef.current = true;
    try {
      // Set by the cycle when the restore gave up after its retry: the
      // engine is stuck on a probed candidate. This flag, not the
      // status-state check below, is what aborts the escalation in that
      // corner: status state can lag the real engine by a poll tick.
      // A holder object, not a let: TS control-flow analysis does not
      // trust assignments made only inside a callback for a plain let.
      const restoreFailureHolder: {
        current: { originalId: string; currentId: string } | null;
      } = { current: null };
      const results: StrategyProbeResult[] = await runHeadlessAutoTune({
        candidates: AUTO_TUNE_CANDIDATES,
        getStatus: () =>
          invoke<SupervisorStatusResponse>("get_supervisor_status"),
        startStrategy: (id) =>
          optsRef.current.onApplyStrategy(id, { silent: true }),
        stopEngine: () => invoke("stop_supervisor"),
        setLastStrategy: (id) =>
          invoke("set_last_strategy", { strategyId: id }),
        isCancelled: () => isAutoTuneModalOpen(),
        onRestoreFailed: (failure) => {
          restoreFailureHolder.current = failure;
        },
      });
      const o3 = optsRef.current;
      const failure = restoreFailureHolder.current;
      if (failure) {
        const tt = tRef.current;
        const origName =
          o3.strategies.find((s) => s.id === failure.originalId)?.name ??
          failure.originalId;
        const curName =
          o3.strategies.find((s) => s.id === failure.currentId)?.name ??
          failure.currentId;
        await notifyUser(
          tt.notify.watchdogRestoreFailedTitle,
          tt.notify.watchdogRestoreFailedBody
            .replace("{from}", origName)
            .replace("{to}", curName)
        );
        return;
      }
      // The cycle took minutes: re-validate everything before touching
      // the engine. A user switch, stop, disable, or an opened modal
      // during the cycle wins over the watchdog, unconditionally.
      if (
        !o3.enabled ||
        o3.engineState !== "running" ||
        o3.activeStrategyId !== triggerId ||
        isAutoTuneModalOpen()
      ) {
        return;
      }
      const winner = pickWinner(results);
      const currentId = o3.activeStrategyId;
      // Baseline: when the present strategy was itself probed in this
      // cycle, its fresh cycle score replaces the minutes-old trigger
      // report score, so a recovered present cannot lose to an equal
      // candidate. Both scores use the same YouTube/Discord mapping
      // (Operational 2, Degraded 1, anything else 0), so they compare
      // directly. A failed probe of the present (engine not running)
      // is not a usable baseline: fall back to the trigger report.
      const freshCurrent = results.find(
        (r) => r.strategy_id === currentId
      );
      const currentScore =
        freshCurrent && freshCurrent.engine_running
          ? probeYtDiscScore(freshCurrent)
          : ytDiscScore(triggerReport);
      if (
        !winner ||
        winner.strategy_id === currentId ||
        probeYtDiscScore(winner) <= currentScore
      ) {
        return;
      }
      const fromName =
        o3.strategies.find((s) => s.id === currentId)?.name ?? currentId;
      await o3.onApplyStrategy(winner.strategy_id, { silent: true });
      // handleStartStrategy can silently no-op (engine transition guard),
      // so toast only after verifying the engine actually moved.
      const st = await invoke<SupervisorStatusResponse>(
        "get_supervisor_status"
      );
      if (st.supervisor.active_strategy !== winner.strategy_id) return;
      failuresRef.current = 0;
      const tt = tRef.current;
      const body = fromName
        ? tt.notify.watchdogSwitchedBody
            .replace("{services}", bad.join(", "))
            .replace("{from}", fromName)
            .replace("{to}", winner.strategy_name)
        : tt.notify.watchdogSwitchedBodyNoFrom
            .replace("{services}", bad.join(", "))
            .replace("{to}", winner.strategy_name);
      await notifyUser(tt.notify.watchdogSwitchedTitle, body);
    } catch (err) {
      console.error("Watchdog escalation error:", err);
    } finally {
      escalatingRef.current = false;
    }
  }
}
