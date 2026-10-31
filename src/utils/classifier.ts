import { StrategyProbeResult } from "../types";

/**
 * Gate for offering the block classifier after a failed auto-tune
 * (design §5.2). The hint is shown only when the failure is the honest
 * kind — every candidate was measured and scored zero — and never when
 * the probes themselves did not happen (engine never started, commands
 * failed): in that case there is nothing to classify, and the existing
 * results table with its errors stays as-is.
 *
 * Pure function over the front-end mirror of the probe results, so the
 * cases are checkable on fixtures:
 * - a winner exists            -> false (the hint lives in the no-winner branch anyway)
 * - empty results               -> false
 * - every result has an error
 *   or engine_running == false  -> false
 * - at least one result with
 *   engine_running && !error    -> true
 */
export function shouldOfferClassifier(
  results: StrategyProbeResult[] | null | undefined,
  winner: StrategyProbeResult | null | undefined,
): boolean {
  if (winner) return false;
  if (!results || results.length === 0) return false;
  return results.some((r) => r.engine_running === true && r.error == null);
}
