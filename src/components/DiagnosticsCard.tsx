import React from "react";
import { Youtube, MessageSquare, Activity, AlertCircle, CheckCircle2, XCircle, RefreshCw } from "lucide-react";
import { DiagnosticsReport, ProbeStatus } from "../types";
import { useI18n } from "../i18n";

interface DiagnosticsCardProps {
  report: DiagnosticsReport;
  onRefresh?: () => void;
  isRefreshing?: boolean;
}

export const DiagnosticsCard: React.FC<DiagnosticsCardProps> = React.memo(({ report, onRefresh, isRefreshing }) => {
  const { t } = useI18n();

  const getStatusBadge = (status: ProbeStatus) => {
    switch (status) {
      case "operational":
        return (
          <span className="flex items-center gap-1 rounded-full bg-emerald-500/10 border border-emerald-500/20 px-2.5 py-0.5 text-xs font-semibold text-emerald-400">
            <CheckCircle2 className="h-3 w-3" />
            {t.diagnostics.operational}
          </span>
        );
      case "degraded":
        return (
          <span className="flex items-center gap-1 rounded-full bg-amber-500/10 border border-amber-500/20 px-2.5 py-0.5 text-xs font-semibold text-amber-400">
            <AlertCircle className="h-3 w-3" />
            {t.diagnostics.degraded}
          </span>
        );
      case "blocked":
        return (
          <span className="flex items-center gap-1 rounded-full bg-rose-500/10 border border-rose-500/20 px-2.5 py-0.5 text-xs font-semibold text-rose-400">
            <XCircle className="h-3 w-3" />
            {t.diagnostics.blocked}
          </span>
        );
      case "checking":
        return (
          <span className="flex items-center gap-1 rounded-full bg-slate-500/10 border border-slate-500/20 px-2.5 py-0.5 text-xs font-semibold text-slate-400">
            <Activity className="h-3 w-3 animate-pulse" />
            {t.diagnostics.checking}
          </span>
        );
    }
  };

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
      {/* YouTube Probe Card */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4 transition-all hover:border-slate-700/80">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-red-500/15 text-red-400">
              <Youtube className="h-4 w-4" />
            </div>
            <div>
              <div className="text-sm font-semibold text-white">{t.diagnostics.youtubeTitle}</div>
              <div className="text-[11px] text-slate-400">{t.diagnostics.youtubeSubtitle}</div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {getStatusBadge(report.youtube.status)}
            {onRefresh && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onRefresh();
                }}
                disabled={isRefreshing}
                title={t.diagnostics.refreshTooltip}
                className="p-1 rounded-md text-slate-400 hover:text-white hover:bg-slate-800 transition-colors disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isRefreshing ? "animate-spin text-cyan-400" : ""}`} />
              </button>
            )}
          </div>
        </div>

        <div className="flex items-baseline justify-between border-t border-slate-800/80 pt-3 text-xs">
          <span className="text-slate-400">{t.diagnostics.latency}</span>
          <span className="font-mono font-medium text-slate-200">
            {report.youtube.latency_ms !== null ? `${report.youtube.latency_ms} ms` : "—"}
          </span>
        </div>

        <div className="mt-1 flex items-baseline justify-between text-xs">
          <span className="text-slate-400">{t.diagnostics.packetLoss}</span>
          <span className="font-mono font-medium text-slate-200">
            {report.youtube.packet_loss_pct}%
          </span>
        </div>

        <div className="mt-2 text-[11px] text-slate-400 truncate" title={report.youtube.message}>
          {report.youtube.message}
        </div>
      </div>

      {/* Discord Voice & Gateway Card */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4 transition-all hover:border-slate-700/80">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-indigo-500/15 text-indigo-400">
              <MessageSquare className="h-4 w-4" />
            </div>
            <div>
              <div className="text-sm font-semibold text-white">{t.diagnostics.discordTitle}</div>
              <div className="text-[11px] text-slate-400">{t.diagnostics.discordSubtitle}</div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {getStatusBadge(report.discord.status)}
            {onRefresh && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onRefresh();
                }}
                disabled={isRefreshing}
                title={t.diagnostics.refreshTooltip}
                className="p-1 rounded-md text-slate-400 hover:text-white hover:bg-slate-800 transition-colors disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${isRefreshing ? "animate-spin text-cyan-400" : ""}`} />
              </button>
            )}
          </div>
        </div>

        <div className="flex items-baseline justify-between border-t border-slate-800/80 pt-3 text-xs">
          <span className="text-slate-400">{t.diagnostics.latency}</span>
          <span className="font-mono font-medium text-slate-200">
            {report.discord.latency_ms !== null ? `${report.discord.latency_ms} ms` : "—"}
          </span>
        </div>

        <div className="mt-1 flex items-baseline justify-between text-xs">
          <span className="text-slate-400">{t.diagnostics.packetLoss}</span>
          <span className="font-mono font-medium text-slate-200">
            {report.discord.packet_loss_pct}%
          </span>
        </div>

        <div className="mt-2 text-[11px] text-slate-400 truncate" title={report.discord.message}>
          {report.discord.message}
        </div>
      </div>
    </div>
  );
});
