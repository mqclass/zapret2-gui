/**
 * Watchdog (strategy guard): background re-checks of the current strategy
 * and automatic escalation to a better one on confirmed degradation.
 *
 * Design constraints, all load-bearing:
 * - Light checks use `refresh_probes` only: plain HTTP through the already
 *   running engine. `probe_strategy` is NEVER used for checks: it restarts
 *   the engine per strategy and would drop connections on every tick.
 * - Degradation counts only YouTube/Discord. The voice probe is an HTTPS
 *   regions endpoint that reads Degraded even when real UDP voice works,
 *   so it must not trigger escalation on its own.
 * - The probe-cycle lock keeps the watchdog's escalation cycle and the
 *   manual auto-tune modal from interleaving probes (each probe restarts
 *   the engine; two loops would attribute results to the wrong strategy).
 * - The headless cycle below mirrors DiagnosticsCard's manual cycle
 *   (pre-check, per-candidate probe_strategy, restore original state)
 *   without its modal UI. The candidate list itself is NOT duplicated:
 *   it is imported from DiagnosticsCard (AUTO_TUNE_CANDIDATES).
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  DiagnosticsReport,
  ProbeStatus,
  StrategyProbeResult,
  SupervisorStatusResponse,
} from "../types";
import { formatErrorMessage } from "./error";

/* Shared probe-cycle state (module-level: App, DiagnosticsCard and the
 * watchdog hook all run in the same webview). */

let autoTuneModalOpen = false;

export function setAutoTuneModalOpenFlag(open: boolean): void {
  autoTuneModalOpen = open;
}

export function isAutoTuneModalOpen(): boolean {
  return autoTuneModalOpen;
}

let probeCycleRunning = false;

export function isProbeCycleRunning(): boolean {
  return probeCycleRunning;
}

export function setProbeCycleRunning(running: boolean): void {
  probeCycleRunning = running;
}

/** Auto-tune scoring: Operational 2, Degraded 1, anything else 0. */
export function scoreStatus(status: ProbeStatus): number {
  return status === "operational" ? 2 : status === "degraded" ? 1 : 0;
}

/**
 * YouTube/Discord subscore (0-4) for a light report. Voice is excluded
 * on purpose: the voice probe reads an HTTPS endpoint that reports
 * Degraded even when real UDP voice works, so it must not decide
 * whether a strategy counts as degraded or which candidate wins.
 * (A candidate fixing YouTube and Discord but reading voice degraded
 * would otherwise tie with the degraded present and never be applied.)
 */
export function ytDiscScore(report: DiagnosticsReport): number {
  return scoreStatus(report.youtube.status) + scoreStatus(report.discord.status);
}

/** Same YouTube/Discord subscore for a probe-cycle result. */
export function probeYtDiscScore(result: StrategyProbeResult): number {
  return (
    scoreStatus(result.youtube_status) + scoreStatus(result.discord_status)
  );
}

/**
 * Services that count toward degradation. "checking" is not degradation:
 * a report mid-flight must not count as a failed check.
 */
export function degradedServices(report: DiagnosticsReport): string[] {
  const out: string[] = [];
  if (
    report.youtube.status === "degraded" ||
    report.youtube.status === "blocked"
  ) {
    out.push("YouTube");
  }
  if (
    report.discord.status === "degraded" ||
    report.discord.status === "blocked"
  ) {
    out.push("Discord");
  }
  return out;
}

/**
 * Cycle winner for the watchdog: highest YouTube/Discord subscore, then
 * lowest average ping. Voice is deliberately not ranked: the voice probe
 * is an HTTPS endpoint, not real UDP voice, and must not outrank a
 * candidate that fixed YouTube and Discord. Requires a positive
 * YouTube/Discord subscore: a candidate that fixes nothing is not a win.
 */
export function pickWinner(
  results: StrategyProbeResult[]
): StrategyProbeResult | null {
  const usable = results.filter(
    (r) => r.engine_running && probeYtDiscScore(r) > 0
  );
  const sorted = [...usable].sort(
    (a, b) =>
      probeYtDiscScore(b) - probeYtDiscScore(a) ||
      (a.average_ping_ms ?? Number.MAX_SAFE_INTEGER) -
        (b.average_ping_ms ?? Number.MAX_SAFE_INTEGER)
  );
  return sorted[0] ?? null;
}

export interface HeadlessCycleDeps {
  candidates: string[];
  getStatus: () => Promise<SupervisorStatusResponse>;
  startStrategy: (id: string) => Promise<void>;
  stopEngine: () => Promise<void>;
  setLastStrategy: (id: string | null) => Promise<void>;
  /** Abort between probes (e.g. the user opened the manual modal). */
  isCancelled: () => boolean;
  /**
   * Restore gave up: the engine is stuck on `currentId` instead of the
   * original strategy. The caller must tell the user and must not apply
   * a winner on top of the broken state.
   */
  onRestoreFailed?: (failure: {
    originalId: string;
    currentId: string;
  }) => void;
}

/**
 * Headless auto-tune cycle: probe every candidate with probe_strategy,
 * then restore the original engine state exactly like the manual modal
 * does. Returns the per-candidate results (empty when the pre-check
 * aborts). The caller decides whether to apply a winner.
 */
export async function runHeadlessAutoTune(
  deps: HeadlessCycleDeps
): Promise<StrategyProbeResult[]> {
  // Claimed synchronously before the first await: whoever claims the lock
  // first runs, the other side (manual modal or watchdog tick) backs off.
  setProbeCycleRunning(true);
  try {
    // Pre-check: read the starting state for the restore step. Starting /
    // Stopping are waited out (up to ~5s); anything else aborts, since a
    // probe started mid-transition would measure the wrong engine.
    let wasRunning = false;
    let originalId: string | null = null;
    try {
      let st = await deps.getStatus();
      for (
        let i = 0;
        i < 10 &&
        (st.supervisor.state === "starting" ||
          st.supervisor.state === "stopping");
        i++
      ) {
        await new Promise((r) => setTimeout(r, 500));
        st = await deps.getStatus();
      }
      if (
        st.supervisor.state === "starting" ||
        st.supervisor.state === "stopping"
      ) {
        return [];
      }
      wasRunning = st.supervisor.state === "running";
      originalId = st.supervisor.active_strategy ?? null;
    } catch (err) {
      console.error("Watchdog cycle pre-check error:", err);
      return [];
    }

    const results: StrategyProbeResult[] = [];
    let probedAny = false;
    let lastStartedId: string | null = null;
    try {
      for (const id of deps.candidates) {
        if (deps.isCancelled()) break;
        probedAny = true;
        lastStartedId = id;
        try {
          const res = await invoke<StrategyProbeResult>("probe_strategy", {
            strategyId: id,
          });
          results.push(res);
          if (res.error && res.error.includes("ADMIN_REQUIRED")) break;
        } catch (err) {
          console.error("Watchdog probe error:", err);
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
        }
      }
    } finally {
      // Restore only what the loop itself left behind. The user may have
      // intervened while a probe was in flight: re-read the state and
      // never override their choice.
      if (probedAny) {
        try {
          const current = await deps.getStatus();
          const curState = current.supervisor.state;
          const curStrategy = current.supervisor.active_strategy ?? null;
          if (wasRunning && originalId) {
            if (
              curState === "running" &&
              curStrategy === lastStartedId &&
              lastStartedId !== originalId
            ) {
              // One retry after a short delay: a transient start failure
              // must not strand the engine on a probed candidate. If
              // both attempts fail, report it instead of staying silent:
              // the caller aborts the escalation and tells the user
              // which strategy is actually active now.
              let restored = false;
              for (let attempt = 0; attempt < 2 && !restored; attempt++) {
                try {
                  await deps.startStrategy(originalId);
                  restored = true;
                } catch (err) {
                  console.error("Watchdog restore start error:", err);
                  if (attempt === 0) {
                    await new Promise((r) => setTimeout(r, 500));
                  }
                }
              }
              if (!restored) {
                deps.onRestoreFailed?.({
                  originalId,
                  currentId: lastStartedId ?? originalId,
                });
              }
            }
            // Already on the original, or the user stopped/switched: leave it.
          } else if (!wasRunning) {
            if (curState === "running" && curStrategy === lastStartedId) {
              try {
                await deps.stopEngine();
              } catch (err) {
                console.error("Watchdog restore stop error:", err);
              }
              try {
                await deps.setLastStrategy(originalId);
              } catch (err) {
                console.error("Watchdog restore settings error:", err);
              }
            } else if (curState === "stopped" || curState === "stopping") {
              try {
                await deps.setLastStrategy(originalId);
              } catch (err) {
                console.error("Watchdog restore settings error:", err);
              }
            }
            // Running on another strategy or starting: the user took over.
          } else {
            // Running, but no original strategy was recorded (the engine
            // started with no active strategy): there is nothing safe to
            // restore to, so leave it as-is — but say so in the log
            // instead of silently no-op'ing the restore step.
            console.warn(
              "Watchdog restore: no original strategy recorded, leaving engine as-is"
            );
          }
        } catch (err) {
          console.error("Watchdog restore error:", err);
        }
      }
    }
    return results;
  } finally {
    setProbeCycleRunning(false);
  }
}
