import React from "react";
import { Activity, AlertCircle, CheckCircle2, XCircle } from "lucide-react";
import { ProbeStatus } from "../types";
import { useI18n } from "../i18n";

/** Small pill describing a probe outcome (Работает / Замедлено / …).
 *  Shared by the diagnostics cards and the auto-tune results modal. */
export const ProbeStatusBadge: React.FC<{ status: ProbeStatus }> = React.memo(
  ({ status }) => {
    const { t } = useI18n();
    switch (status) {
      case "operational":
        return (
          <span className="flex items-center gap-1 rounded-full bg-emerald-500/15 border border-emerald-500/30 px-2 py-0.5 text-[11px] font-semibold text-emerald-400">
            <CheckCircle2 className="h-3 w-3" />
            {t.diagnostics.operational}
          </span>
        );
      case "degraded":
        return (
          <span className="flex items-center gap-1 rounded-full bg-amber-500/15 border border-amber-500/30 px-2 py-0.5 text-[11px] font-semibold text-amber-400">
            <AlertCircle className="h-3 w-3" />
            {t.diagnostics.degraded}
          </span>
        );
      case "blocked":
        return (
          <span className="flex items-center gap-1 rounded-full bg-rose-500/15 border border-rose-500/30 px-2 py-0.5 text-[11px] font-semibold text-rose-400">
            <XCircle className="h-3 w-3" />
            {t.diagnostics.blocked}
          </span>
        );
      case "checking":
      default:
        return (
          <span className="flex items-center gap-1 rounded-full bg-ink5/15 border border-ink5/30 px-2 py-0.5 text-[11px] font-semibold text-ink4">
            <Activity className="h-3 w-3 animate-pulse text-accent" />
            {t.diagnostics.checking}
          </span>
        );
    }
  }
);
