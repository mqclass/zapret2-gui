import React, { useEffect, useRef, useState } from "react";
import { LogViewer, formatLogTime, formatLogsForClipboard } from "../LogViewer";
import { LogEntry, LuaStrategy } from "../../types";
import { Check, Copy, Terminal } from "lucide-react";
import { useI18n } from "../../i18n";
import { humanizeLogMessage } from "../../utils/humanizeLog";

interface LogsViewProps {
  logs: LogEntry[];
  onClearLogs: () => void;
  onRefreshLogs?: () => void;
  isRefreshing?: boolean;
  /** Redesign stage 5: "simple" renders a digest of the last events
   * in plain rows instead of the raw terminal with filters. */
  variant?: "full" | "simple";
  /** Switch the shell to expert mode (the digest's "show full log"). */
  onShowExpert?: () => void;
  /** Strategy catalog for human names in the simple digest; ids pass
   * through when the catalog is absent or an id is unknown. */
  strategies?: LuaStrategy[];
}

/** How many recent events the simple digest shows. */
const DIGEST_COUNT = 5;

export const LogsView: React.FC<LogsViewProps> = React.memo(
  ({ logs, onClearLogs, onRefreshLogs, isRefreshing, variant = "full", onShowExpert, strategies }) => {
    const { t, language } = useI18n();
    const [copied, setCopied] = useState(false);
    const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    useEffect(() => {
      return () => {
        if (copiedTimerRef.current) {
          clearTimeout(copiedTimerRef.current);
        }
      };
    }, []);

    if (variant === "simple") {
      const recent = logs.slice(-DIGEST_COUNT);
      const levelWord = (level: LogEntry["level"]) =>
        level === "error" || level === "stderr"
          ? { word: t.diagTab.levelError, dot: "bg-rose-400" }
          : level === "warn"
            ? { word: t.diagTab.levelWarn, dot: "bg-amber-400" }
            : { word: t.diagTab.levelInfo, dot: "bg-accent" };
      const handleCopy = async () => {
        try {
          await navigator.clipboard.writeText(formatLogsForClipboard(logs));
          setCopied(true);
          if (copiedTimerRef.current) {
            clearTimeout(copiedTimerRef.current);
          }
          copiedTimerRef.current = setTimeout(() => setCopied(false), 2000);
        } catch (e) {
          console.error("Failed to copy diagnostics:", e);
        }
      };

      return (
        <div className="rounded-panel border border-line bg-card p-3.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <div className="flex h-7 w-7 items-center justify-center rounded-control bg-emerald-500/10 border border-emerald-500/25 text-emerald-400">
                <Terminal className="h-4 w-4" />
              </div>
              <h2 className="text-[13px] font-bold text-ink">{t.diagTab.logDigestTitle}</h2>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleCopy}
                className="flex items-center gap-1.5 rounded-control border border-line bg-cardin px-2.5 py-1 text-xs text-ink3 hover:bg-hovert hover:text-ink transition-all cursor-pointer"
              >
                {copied ? (
                  <Check className="h-3.5 w-3.5 text-emerald-400" />
                ) : (
                  <Copy className="h-3.5 w-3.5 text-ink4" />
                )}
                <span>{copied ? t.diagTab.copied : t.diagTab.copyDiagnostics}</span>
              </button>
              {onShowExpert && (
                <button
                  type="button"
                  onClick={onShowExpert}
                  className="text-xs text-accent-soft hover:text-ink transition-colors cursor-pointer"
                >
                  {t.diagTab.showFullLog} →
                </button>
              )}
            </div>
          </div>

          {recent.length === 0 ? (
            <p className="mt-2 text-xs text-ink4">{t.diagTab.logDigestEmpty}</p>
          ) : (
            <ul className="mt-2 space-y-1">
              {recent.map((log) => {
                const { word, dot } = levelWord(log.level);
                return (
                  <li
                    key={log.id ?? `${log.timestamp}-${log.message}`}
                    className="flex items-center gap-2.5 text-[11px] leading-relaxed"
                  >
                    <span className="text-ink5 shrink-0 font-mono">
                      {formatLogTime(log.timestamp)}
                    </span>
                    <span className="flex items-center gap-1.5 shrink-0">
                      <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
                      <span className="text-ink3">{word}</span>
                    </span>
                    <span
                      className="text-ink2 truncate"
                      title={log.message}
                    >
                      {humanizeLogMessage(log.message, t, language, strategies)}
                    </span>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      );
    }

    return (
      <div className="space-y-4 pb-6">
        <LogViewer
          logs={logs}
          onClearLogs={onClearLogs}
          onRefreshLogs={onRefreshLogs}
          isRefreshing={isRefreshing}
          fullHeight={true}
        />
      </div>
    );
  }
);
