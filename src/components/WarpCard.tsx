import React, { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Cloud, Play, Square, Loader2, AlertTriangle, RefreshCw } from "lucide-react";
import { SingBoxStatus, TunnelState, TransportView } from "../types";
import { useI18n } from "../i18n";

interface WarpCardProps {
  /** Redesign stage 4: "simple" is one compact card (status + toggle);
      "full" adds the expert details (engine version, health port,
      exit delay), shown in expert mode. */
  variant?: "simple" | "full";
  /** Switch the shell to expert mode (simple card's "more" action). */
  onShowExpert?: () => void;
}

export const WarpCard: React.FC<WarpCardProps> = React.memo(
  ({ variant = "full", onShowExpert }) => {
    const { t } = useI18n();
    const [status, setStatus] = useState<SingBoxStatus | null>(null);
    const [warpTransport, setWarpTransport] = useState<TransportView | null>(null);
    const [busy, setBusy] = useState(false);
    const [retrying, setRetrying] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const refresh = useCallback(async () => {
      try {
        const s = await invoke<SingBoxStatus>("get_tunnel_status");
        setStatus(s);
        // A local error from a transient invoke failure (e.g. the
        // remote start was accepted but its read-back failed) must
        // not stick once the polled state is fine: a real backend
        // failure still surfaces through last_error while the state
        // is "error" (stage 5 review m3).
        if (s.state !== "error") {
          setError(null);
        }
      } catch (e) {
        console.error("Failed to fetch tunnel status:", e);
      }
    }, []);

    const refreshAvailability = useCallback(async () => {
      try {
        const transports = await invoke<TransportView[]>("list_transports");
        setWarpTransport(transports.find((v) => v.id === "warp") ?? null);
      } catch (e) {
        // Unknown availability: do not block the toggle - the backend
        // refuses honestly when the binaries are missing.
        console.error("Failed to fetch transports:", e);
      }
    }, []);

    useEffect(() => {
      refresh();
      refreshAvailability();
      const unlistenPromise = listen<TunnelState>("tunnel-state", () => {
        refresh();
      });
      const timer = setInterval(refresh, 5000);
      return () => {
        clearInterval(timer);
        unlistenPromise.then((unlisten) => unlisten());
      };
    }, [refresh, refreshAvailability]);

    const state: TunnelState = status?.state ?? "stopped";
    const isOn =
      state === "running" || state === "degraded" || state === "unregistered";
    const isTransitioning = state === "starting" || state === "stopping";
    const unavailable = warpTransport !== null && !warpTransport.available;

    const statusLabel =
      state === "running"
        ? t.warp.statusWorking
        : state === "starting"
          ? t.warp.statusConnecting
          : state === "stopping"
            ? t.warp.statusStopping
            : state === "degraded"
              ? t.warp.statusDegraded
              : state === "unregistered"
                ? t.warp.statusUnregistered
                : state === "error"
                  ? t.warp.statusError
                  : t.warp.statusOff;
    const statusBadgeClass =
      state === "running"
        ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/30"
        : state === "error"
          ? "bg-rose-500/15 text-rose-300 border-rose-500/30"
          : state === "stopped"
            ? "bg-hovert text-ink4 border-line"
            : "bg-amber-500/15 text-amber-300 border-amber-500/30";

    const handleToggle = useCallback(async () => {
      if (busy || retrying || isTransitioning || unavailable) return;
      setBusy(true);
      setError(null);
      try {
        if (isOn) {
          await invoke("tunnel_stop");
        } else {
          await invoke<SingBoxStatus>("tunnel_start");
        }
        await refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    }, [busy, retrying, isTransitioning, unavailable, isOn, refresh]);

    // Manual retry (stage 6): shown on Error and on Unregistered.
    // On Error it is the explicit affordance after the auto-restart
    // budget is spent; on Unregistered it is the only retry path -
    // the main button there is "Disable", and a re-registration
    // needs a real restart, not a toggle.
    const showRetry = state === "error" || state === "unregistered";
    const handleRetry = useCallback(async () => {
      if (retrying || busy || isTransitioning || unavailable) return;
      setRetrying(true);
      setError(null);
      try {
        // A restart, not a plain start: stop clears the Error state
        // (and kills a live but unregistered engine), then start
        // runs the full path with a fresh restart budget.
        await invoke("tunnel_stop");
        await invoke<SingBoxStatus>("tunnel_start");
        await refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setRetrying(false);
      }
    }, [retrying, busy, isTransitioning, unavailable, refresh]);

    // The engine version line is "sing-box version 1.14.3-...": the
    // prefix adds nothing in the UI, show the version only.
    const engineVersion = status?.engine_version
      ? status.engine_version.replace(/^sing-box version\s+/i, "")
      : null;

    const showError = error ?? (state === "error" ? status?.last_error ?? null : null);

    return (
      <div
        className={
          variant === "simple"
            ? "t-card p-4 space-y-2.5"
            : "t-card p-5 space-y-4 stagger-item"
        }
        style={
          variant === "simple"
            ? undefined
            : ({ "--stagger-i": 2 } as React.CSSProperties)
        }
      >
        <div
          className={
            variant === "simple"
              ? "flex items-center justify-between gap-3"
              : "flex items-center justify-between gap-3 border-b border-line pb-3"
          }
        >
          <div className="flex items-center gap-3 min-w-0">
            <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-sky-500/20 to-sky-600/5 border border-sky-500/25 text-sky-400 shadow-sm shadow-sky-500/5">
              <Cloud className="h-4 w-4 shrink-0" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <h3 className="text-xs font-semibold text-ink">{t.warp.title}</h3>
                <span className="rounded-chip px-1.5 py-0.5 text-[9px] font-semibold uppercase border shrink-0 bg-sky-500/15 text-sky-300 border-sky-500/30">
                  {t.warp.experimentalBadge}
                </span>
              </div>
              {variant === "full" && (
                <p className="text-[11px] text-ink4 leading-snug mt-0.5">{t.warp.hint}</p>
              )}
            </div>
          </div>
          <span
            className={`rounded-chip px-1.5 py-0.5 text-[9px] font-semibold uppercase border shrink-0 ${unavailable ? "bg-hovert text-ink4 border-line" : statusBadgeClass}`}
          >
            {unavailable ? t.warp.statusUnavailable : statusLabel}
          </span>
        </div>

        {variant === "simple" && (
          <p className="text-[11px] text-ink4 leading-relaxed">{t.warp.hint}</p>
        )}

        {unavailable && warpTransport?.unavailable_reason && (
          <div className="flex items-start gap-2 rounded-panel border border-line bg-cardin p-3 text-xs text-ink3">
            <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5 text-amber-400" />
            <span>{warpTransport.unavailable_reason}</span>
          </div>
        )}

        {showError && (
          <div className="flex items-start gap-2 rounded-panel border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5 text-rose-400" />
            <span className="break-words">{showError}</span>
          </div>
        )}

        {status && status.restarts_used > 0 && (
          <div className="flex items-center gap-1.5 text-[11px] text-ink4">
            <RefreshCw className="h-3 w-3 shrink-0" />
            <span>
              {t.warp.restartsLabel}: {status.restarts_used}
              {status.restarts_max > 0 ? ` / ${status.restarts_max}` : ""}
            </span>
          </div>
        )}

        {variant === "full" && status && state !== "stopped" && (
          <div className="grid grid-cols-3 gap-2 text-[11px]">
            <div className="rounded-panel border border-line bg-cardin p-2.5 space-y-0.5 min-w-0 flex flex-col">
              <div className="text-ink5 text-[10px]">{t.warp.engineVersionLabel}</div>
              <div
                className="font-mono text-[10px] text-ink3 break-words mt-auto"
                title={status.engine_version ?? ""}
              >
                {engineVersion ?? "—"}
              </div>
            </div>
            <div className="rounded-panel border border-line bg-cardin p-2.5 space-y-0.5 min-w-0 flex flex-col">
              <div className="text-ink5 text-[10px]">{t.warp.clashPortLabel}</div>
              <div className="font-mono text-ink3 truncate mt-auto">
                {status.clash_api_port !== null ? String(status.clash_api_port) : "—"}
              </div>
            </div>
            <div className="rounded-panel border border-line bg-cardin p-2.5 space-y-0.5 min-w-0 flex flex-col">
              <div className="text-ink5 text-[10px]">{t.warp.delayLabel}</div>
              <div className="font-mono text-ink3 truncate mt-auto">
                {status.warmup_delay_ms !== null ? `${status.warmup_delay_ms} ms` : "—"}
              </div>
            </div>
          </div>
        )}

        <div className="flex items-center gap-2 flex-wrap">
          <button
            type="button"
            onClick={handleToggle}
            disabled={busy || retrying || isTransitioning || unavailable || status === null}
            className={`t-btn ${isOn ? "t-btn-danger" : "t-btn-primary"}`}
          >
            {busy || isTransitioning ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : isOn ? (
              <Square className="h-4 w-4" />
            ) : (
              <Play className="h-4 w-4" />
            )}
            {busy
              ? isOn
                ? t.warp.disabling
                : t.warp.enabling
              : isOn
                ? t.warp.disableBtn
                : t.warp.enableBtn}
          </button>
          {showRetry && (
            <button
              type="button"
              onClick={handleRetry}
              disabled={retrying || busy || isTransitioning || unavailable || status === null}
              className="t-btn t-btn-ghost"
            >
              {retrying ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              {retrying ? t.warp.retrying : t.warp.retryBtn}
            </button>
          )}
          {variant === "simple" && onShowExpert && (
            <button
              type="button"
              onClick={onShowExpert}
              className="text-[11px] font-medium text-accent-soft hover:text-accent transition-colors cursor-pointer ml-auto"
            >
              {t.warp.detailsLink} →
            </button>
          )}
        </div>
      </div>
    );
  }
);
