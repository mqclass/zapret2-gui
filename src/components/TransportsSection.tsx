import React, { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  FlaskConical,
  Check,
  AlertTriangle,
  Loader2,
  Network,
} from "lucide-react";
import { useI18n } from "../i18n";
import { TransportView } from "../types";
import { formatErrorMessage } from "../utils/error";
import { notifyUser } from "../utils/notify";

/** Localized display name for known transport ids; falls back to the
 *  backend-provided English name for third-party transports. */
function transportName(t: ReturnType<typeof useI18n>["t"], v: TransportView): string {
  if (v.id === "direct") return t.transports.name_direct;
  if (v.id === "domain-fronting") return t.transports.name_domain_fronting;
  if (v.id === "warp") return t.transports.name_warp;
  return v.name;
}

function transportDesc(t: ReturnType<typeof useI18n>["t"], v: TransportView): string {
  if (v.id === "direct") return t.transports.desc_direct;
  if (v.id === "domain-fronting") return t.transports.desc_domain_fronting;
  if (v.id === "warp") return t.transports.desc_warp;
  return v.description;
}

/**
 * "Transports (experimental)" section.
 *
 * Self-contained: enumerates the backend TransportRegistry over IPC
 * (`list_transports`), shows live availability with explicit refusal reasons,
 * and persists the choice via `set_active_transport`. Experimental and
 * unavailable transports are visually marked and can never be enabled
 * silently — the backend refuses them with a reason.
 */
export const TransportsSection: React.FC = React.memo(() => {
  const { t } = useI18n();
  const [transports, setTransports] = useState<TransportView[] | null>(null);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectingId, setSelectingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [list, active] = await Promise.all([
        invoke<TransportView[]>("list_transports"),
        invoke<string | null>("get_active_transport"),
      ]);
      setTransports(list);
      setActiveId(active);
    } catch (err) {
      setError(`${t.transports.loadError}: ${formatErrorMessage(err)}`);
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    load();
  }, [load]);

  const handleSelect = useCallback(
    async (id: string) => {
      setSelectingId(id);
      setError(null);
      try {
        await invoke("set_active_transport", { transportId: id });
        setActiveId(id === "direct" ? null : id);
        const view = transports?.find((v) => v.id === id);
        const name = view ? transportName(t, view) : id;
        notifyUser(
          t.transports.selected,
          t.transports.setSuccess.replace("{name}", name)
        );
      } catch (err) {
        setError(`${t.transports.setError}${formatErrorMessage(err)}`);
      } finally {
        setSelectingId(null);
      }
    },
    [t, transports]
  );

  return (
    <div className="rounded-card border border-line bg-card p-5 space-y-3 shadow-card">
      <div className="border-b border-line pb-3">
        <h3 className="t-section-title">
          <FlaskConical className="h-4 w-4 text-accent" />
          <span>{t.transports.title}</span>
          <span className="rounded-chip px-1.5 py-0.5 text-[10px] font-medium bg-warnwash border border-warnline text-ink2">
            {t.transports.experimentalBadge}
          </span>
        </h3>
      </div>

      <p className="text-[11px] text-ink4 leading-relaxed max-w-2xl">
        {t.transports.description}
      </p>

      {loading && (
        <div className="flex items-center gap-2 text-ink4 text-xs py-4">
          <Loader2 className="h-4 w-4 animate-spin text-accent" />
        </div>
      )}

      {error && (
        <div className="flex items-start gap-2 rounded-panel border border-warnline bg-warnwash p-3">
          <AlertTriangle className="h-4 w-4 text-ink2 shrink-0 mt-0.5" />
          <p className="text-[11px] text-ink3 leading-relaxed">{error}</p>
        </div>
      )}

      {!loading && transports && (
        <div className="space-y-2.5">
          {transports.map((v, idx) => {
            const isActive = (activeId ?? "direct") === v.id;
            const isSelecting = selectingId === v.id;
            const name = transportName(t, v);
            return (
              <div
                key={v.id}
                className={`rounded-panel border p-3.5 space-y-2 transition-all stagger-item hover-lift ${
                  isActive
                    ? "border-accent-line bg-accent-wash"
                    : "border-line bg-cardin"
                }`}
                style={{ "--stagger-i": idx } as React.CSSProperties}
              >
                <div className="flex items-center justify-between gap-3 flex-wrap">
                  <div className="flex items-center gap-2 min-w-0">
                    <Network className="h-4 w-4 text-accent shrink-0" />
                    <span className="text-xs font-semibold text-ink truncate">
                      {name}
                    </span>
                    <span className="font-mono text-[10px] text-ink5 shrink-0">
                      {v.id}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5 flex-wrap">
                    {v.experimental && (
                      <span className="rounded-chip px-1.5 py-0.5 text-[10px] font-medium bg-warnwash border border-warnline text-ink2">
                        {t.transports.experimentalBadge}
                      </span>
                    )}
                    {v.requires_exit_infra && (
                      <span className="rounded-chip px-1.5 py-0.5 text-[10px] font-medium bg-hovert border border-line text-ink4">
                        {t.transports.requiresExitInfra}
                      </span>
                    )}
                    <span
                      className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium border ${
                        v.available
                          ? "bg-accent-wash text-accent-soft border-accent-line"
                          : "bg-hovert text-ink4 border-line"
                      }`}
                    >
                      {v.available
                        ? t.transports.statusAvailable
                        : t.transports.statusUnavailable}
                    </span>
                  </div>
                </div>

                <p className="text-[11px] text-ink4 leading-relaxed">
                  {transportDesc(t, v)}
                </p>

                {!v.available && v.unavailable_reason && (
                  <div className="rounded-panel border border-line bg-hovert p-2.5 space-y-1">
                    <p className="text-[10px] font-semibold uppercase tracking-wide text-ink5">
                      {t.transports.reasonLabel}
                    </p>
                    <p className="text-[11px] text-ink3 leading-relaxed">
                      {v.unavailable_reason}
                    </p>
                    <p className="text-[11px] text-ink4 leading-relaxed">
                      {t.transports.unavailableHint}
                    </p>
                  </div>
                )}

                {v.config_example != null &&
                  typeof v.config_example === "object" &&
                  Object.keys(v.config_example as Record<string, unknown>).length > 0 && (
                    <details className="rounded-panel border border-line bg-hovert px-2.5 py-1.5">
                      <summary className="cursor-pointer text-[11px] text-ink4 hover:text-ink3 select-none">
                        {t.transports.configExampleLabel}
                      </summary>
                      <pre className="mt-1.5 overflow-x-auto font-mono text-[10px] leading-relaxed text-ink3">
                        {JSON.stringify(v.config_example, null, 2)}
                      </pre>
                    </details>
                  )}

                <div className="flex justify-end">
                  {isActive ? (
                    <span className="t-badge t-badge-accent">
                      <Check className="h-3 w-3" />
                      <span className="text-[11px] font-semibold">
                        {t.transports.selected}
                      </span>
                    </span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => handleSelect(v.id)}
                      disabled={isSelecting || !v.available}
                      title={!v.available ? t.transports.unavailableHint : undefined}
                      className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
                    >
                      {isSelecting && (
                        <Loader2 className="h-3.5 w-3.5 animate-spin text-accent" />
                      )}
                      <span>{t.transports.select}</span>
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      <p className="text-[11px] text-ink5 leading-relaxed">{t.transports.docsHint}</p>
    </div>
  );
});
