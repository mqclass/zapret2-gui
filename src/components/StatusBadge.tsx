import React from "react";
import { SupervisorState } from "../types";
import { Play, Square, Loader2, AlertTriangle, ShieldCheck } from "lucide-react";
import { useI18n } from "../i18n";

interface StatusBadgeProps {
  state: SupervisorState;
  pid: number | null;
  activeStrategy: string | null;
  uptimeSecs: number;
}

export const StatusBadge: React.FC<StatusBadgeProps> = React.memo(
  ({ state, pid, activeStrategy, uptimeSecs }) => {
    const { t } = useI18n();

    const formatUptime = (secs: number) => {
      const hrs = Math.floor(secs / 3600);
      const mins = Math.floor((secs % 3600) / 60);
      const s = secs % 60;
      if (hrs > 0) return `${hrs}h ${mins}m ${s}s`;
      if (mins > 0) return `${mins}m ${s}s`;
      return `${s}s`;
    };

    const getStatusConfig = () => {
      switch (state) {
        case "running":
          return {
            label: t.status.active,
            color: "text-emerald-400 bg-emerald-500/10 border-emerald-500/30",
            dotColor: "bg-emerald-400 shadow-emerald-400/50",
            icon: ShieldCheck,
          };
        case "starting":
          return {
            label: t.status.starting,
            color: "text-sky-400 bg-sky-500/10 border-sky-500/30",
            dotColor: "bg-sky-400 shadow-sky-400/50",
            icon: Loader2,
          };
        case "stopping":
          return {
            label: t.status.stopping,
            color: "text-amber-400 bg-amber-500/10 border-amber-500/30",
            dotColor: "bg-amber-400 shadow-amber-400/50",
            icon: Loader2,
          };
        case "error":
          return {
            label: t.status.error,
            color: "text-rose-400 bg-rose-500/10 border-rose-500/30",
            dotColor: "bg-rose-400 shadow-rose-400/50",
            icon: AlertTriangle,
          };
        case "stopped":
        default:
          return {
            label: t.status.stopped,
            color: "text-ink4 bg-raise/60 border-linestrong/60",
            dotColor: "bg-ink5 shadow-transparent",
            icon: Square,
          };
      }
    };

    const config = getStatusConfig();
    const Icon = config.icon;

    return (
      <div className="flex flex-wrap items-center gap-3">
        <div
          className={`flex items-center gap-2 rounded-panel border px-3.5 py-1.5 text-xs font-semibold backdrop-blur-md shadow-sm ${config.color}`}
        >
          <span
            className={`h-2 w-2 rounded-full shadow-md ${
              state === "running" ? "animate-pulse" : ""
            } ${config.dotColor}`}
          />
          <Icon
            className={`h-3.5 w-3.5 ${
              state === "starting" || state === "stopping" ? "animate-spin" : ""
            }`}
          />
          <span>{config.label}</span>
        </div>

        {pid !== null && (
          <div className="flex items-center gap-1.5 rounded-control border border-raise bg-card/50 px-2.5 py-1 text-xs text-ink3 font-mono">
            <span className="text-ink5">{t.status.pid}:</span>
            <span className="font-semibold text-ink2">{pid}</span>
          </div>
        )}

        {state === "running" && (
          <div className="flex items-center gap-1.5 rounded-control border border-raise bg-card/50 px-2.5 py-1 text-xs text-ink3 font-mono">
            <span className="text-ink5">{t.status.uptime}:</span>
            <span className="font-semibold text-ink2">{formatUptime(uptimeSecs)}</span>
          </div>
        )}

        {activeStrategy && (
          <div className="flex items-center gap-1.5 rounded-control border border-indigo-500/20 bg-indigo-500/10 px-2.5 py-1 text-xs text-indigo-300">
            <span className="text-indigo-400/70">{t.status.strategy}:</span>
            <span className="font-medium text-indigo-200">{activeStrategy}</span>
          </div>
        )}
      </div>
    );
  }
);
