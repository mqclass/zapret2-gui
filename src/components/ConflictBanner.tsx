import React from "react";
import { AlertTriangle, Wrench, RefreshCw, ShieldAlert } from "lucide-react";
import { ConflictItem } from "../types";
import { useI18n } from "../i18n";

interface ConflictBannerProps {
  conflicts: ConflictItem[];
  onResolve: () => void;
  isResolving: boolean;
}

export const ConflictBanner: React.FC<ConflictBannerProps> = React.memo(
  ({ conflicts, onResolve, isResolving }) => {
    const { t } = useI18n();

    if (!conflicts || conflicts.length === 0) {
      return null;
    }

    return (
      <div className="rounded-xl border border-amber-500/40 bg-gradient-to-r from-amber-500/15 via-rose-500/10 to-amber-500/15 p-4 text-amber-200 shadow-lg shadow-amber-500/5 backdrop-blur-md">
        <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
          <div className="flex items-start gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-amber-500/20 text-amber-300">
              <ShieldAlert className="h-5 w-5" />
            </div>
            <div>
              <div className="font-semibold text-sm text-amber-100 flex items-center gap-2">
                <span>{t.conflicts.bannerTitle}</span>
                <span className="rounded bg-amber-500/20 px-2 py-0.5 text-[11px] font-mono text-amber-300 border border-amber-500/30">
                  {conflicts.length}
                </span>
              </div>
              <p className="mt-1 text-xs text-amber-200/80 leading-relaxed max-w-2xl">
                {t.conflicts.bannerDesc}
              </p>

              {/* Detected conflicts tags */}
              <div className="mt-2.5 flex flex-wrap items-center gap-2 text-xs">
                <span className="text-amber-400/90 font-medium">{t.conflicts.detectedLabel}</span>
                {conflicts.map((c, i) => (
                  <span
                    key={i}
                    className="inline-flex items-center gap-1 rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-[11px] font-mono text-amber-200"
                  >
                    <span className="font-bold">{c.name}</span>
                    {c.pid && <span className="text-amber-400/70">(PID: {c.pid})</span>}
                    <span className="text-[10px] text-amber-300/60">
                      [{c.kind === "service" ? t.conflicts.serviceKind : t.conflicts.processKind}]
                    </span>
                  </span>
                ))}
              </div>
            </div>
          </div>

          <button
            onClick={onResolve}
            disabled={isResolving}
            className="flex shrink-0 items-center gap-2 rounded-xl border border-amber-400/40 bg-amber-500 hover:bg-amber-400 text-slate-950 font-bold px-4 py-2.5 text-xs shadow-md transition-all active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
          >
            {isResolving ? (
              <>
                <RefreshCw className="h-4 w-4 animate-spin text-slate-950" />
                <span>{t.conflicts.resolving}</span>
              </>
            ) : (
              <>
                <Wrench className="h-4 w-4 text-slate-950" />
                <span>{t.conflicts.resolveBtn}</span>
              </>
            )}
          </button>
        </div>
      </div>
    );
  }
);
