import React, { useState, useEffect, useRef, useMemo } from "react";
import { LogEntry } from "../types";
import {
  Terminal,
  Trash2,
  ArrowDown,
  Search,
  FolderOpen,
  Copy,
  Check,
  RefreshCw,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

interface LogViewerProps {
  logs: LogEntry[];
  onClearLogs: () => void;
  onRefreshLogs?: () => void;
  isRefreshing?: boolean;
  fullHeight?: boolean;
}

export const LogViewer: React.FC<LogViewerProps> = React.memo(
  ({ logs, onClearLogs, onRefreshLogs, isRefreshing = false, fullHeight = false }) => {
    const { t } = useI18n();
    const [filterLevel, setFilterLevel] = useState<string>("all");
    const [searchTerm, setSearchTerm] = useState<string>("");
    const [autoScroll, setAutoScroll] = useState<boolean>(true);
    const [copied, setCopied] = useState<boolean>(false);
    const logContainerRef = useRef<HTMLDivElement>(null);
    const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Clear the "copied" reset timer on unmount.
    useEffect(() => {
      return () => {
        if (copiedTimerRef.current) {
          clearTimeout(copiedTimerRef.current);
        }
      };
    }, []);

    // PERFORMANCE OPTIMIZATION: Memoize filtered and sliced log entries up to 800 lines
    const { filteredCount, displayLogs } = useMemo(() => {
      const term = searchTerm.trim().toLowerCase();
      const filtered = logs.filter((log) => {
        if (filterLevel !== "all" && log.level !== filterLevel) {
          return false;
        }
        if (term && !log.message.toLowerCase().includes(term)) {
          return false;
        }
        return true;
      });
      return {
        filteredCount: filtered.length,
        displayLogs: filtered.slice(-800),
      };
    }, [logs, filterLevel, searchTerm]);

    useEffect(() => {
      if (autoScroll && logContainerRef.current) {
        logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight;
      }
    }, [displayLogs.length, autoScroll]);

    const formatTimestamp = (epoch: number) => {
      const d = new Date(epoch * 1000);
      return d.toTimeString().split(" ")[0];
    };

    const getLevelStyle = (level: string) => {
      switch (level) {
        case "stderr":
        case "error":
          return "text-rose-400 bg-rose-500/10 border border-rose-500/20";
        case "warn":
          return "text-amber-400 bg-amber-500/10 border border-amber-500/20";
        case "info":
          return "text-cyan-400 bg-cyan-500/10 border border-cyan-500/20";
        case "tgproxy":
          return "border border-accent-line bg-accent-wash text-accent";
        case "stdout":
        default:
          return "text-emerald-400 bg-emerald-500/10 border border-emerald-500/20";
      }
    };

    const handleOpenFolder = async () => {
      try {
        await invoke("open_logs_folder");
      } catch (e) {
        console.error("Failed to open logs folder:", e);
      }
    };

    const handleCopyLogs = async () => {
      try {
        const text = logs
          .map((l) => `[${formatTimestamp(l.timestamp)}] [${l.level.toUpperCase()}] ${l.message}`)
          .join("\n");
        await navigator.clipboard.writeText(text);
        setCopied(true);
        if (copiedTimerRef.current) {
          clearTimeout(copiedTimerRef.current);
        }
        copiedTimerRef.current = setTimeout(() => setCopied(false), 2000);
      } catch (e) {
        console.error("Failed to copy logs:", e);
      }
    };

    return (
      <div className="flex flex-col rounded-card border border-white/[0.08] bg-card/40 p-4 shadow-xl backdrop-blur-xl">
        {/* Header & Controls */}
        <div className="flex flex-wrap items-center justify-between gap-3 mb-3.5">
          <div className="flex items-center gap-2.5">
            <div className="flex h-7 w-7 items-center justify-center rounded-control bg-emerald-500/10 border border-emerald-500/25 text-emerald-400">
              <Terminal className="h-4 w-4" />
            </div>
            <h2 className="text-sm font-bold text-ink">{t.logs.title}</h2>
            <span className="rounded-chip bg-white/[0.05] border border-white/[0.08] px-2 py-0.5 text-[11px] font-mono text-ink3">
              {logs.length} {t.logs.lines}
            </span>
            {filteredCount > 800 && (
              <span className="text-[10px] text-ink5">
                ({t.logs.showingLatest})
              </span>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {/* Search Box */}
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-ink5" />
              <input
                type="text"
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                placeholder={t.logs.searchPlaceholder}
                className="h-8 w-36 rounded-panel border border-white/[0.08] bg-cardin/80 pl-8 pr-2.5 text-xs text-ink2 placeholder-ink5 focus:border-cyan-500 focus:outline-none transition-all"
              />
            </div>

            {/* Level Filter */}
            <select
              value={filterLevel}
              onChange={(e) => setFilterLevel(e.target.value)}
              className="h-8 rounded-panel border border-white/[0.08] bg-cardin/80 px-2.5 text-xs text-ink3 focus:border-cyan-500 focus:outline-none cursor-pointer"
            >
              <option value="all">{t.logs.allLevels}</option>
              <option value="stdout">stdout</option>
              <option value="stderr">stderr</option>
              <option value="info">info</option>
              <option value="warn">warn</option>
              <option value="error">error</option>
            </select>

            {/* Manual Refresh Button */}
            {onRefreshLogs && (
              <button
                type="button"
                onClick={onRefreshLogs}
                disabled={isRefreshing}
                className="flex h-8 items-center gap-1.5 rounded-panel border border-white/[0.08] bg-cardin/80 px-2.5 text-xs text-ink3 hover:bg-white/[0.08] hover:text-ink transition-all cursor-pointer disabled:opacity-50"
                title={t.logs.refreshTitle}
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 text-cyan-400 ${isRefreshing ? "animate-spin" : ""}`}
                />
                <span>{t.logs.refreshShort}</span>
              </button>
            )}

            {/* Open Logs Folder */}
            <button
              type="button"
              onClick={handleOpenFolder}
              className="flex h-8 items-center gap-1.5 rounded-panel border border-white/[0.08] bg-cardin/80 px-2.5 text-xs text-ink3 hover:bg-white/[0.08] hover:text-ink transition-all cursor-pointer"
              title={t.logs.openFolder}
            >
              <FolderOpen className="h-3.5 w-3.5 text-cyan-400" />
              <span>{t.logs.openFolder}</span>
            </button>

            {/* Copy Logs */}
            <button
              type="button"
              onClick={handleCopyLogs}
              className="flex h-8 items-center gap-1.5 rounded-panel border border-white/[0.08] bg-cardin/80 px-2.5 text-xs text-ink3 hover:bg-white/[0.08] hover:text-ink transition-all cursor-pointer"
              title={t.logs.copyAll}
            >
              {copied ? (
                <Check className="h-3.5 w-3.5 text-emerald-400" />
              ) : (
                <Copy className="h-3.5 w-3.5 text-ink4" />
              )}
              <span>{copied ? t.logs.copiedSuccess : t.logs.copyAll}</span>
            </button>

            {/* Auto Scroll toggle */}
            <button
              type="button"
              onClick={() => setAutoScroll(!autoScroll)}
              className={`flex h-8 items-center gap-1.5 rounded-panel border px-2.5 text-xs transition-all cursor-pointer ${
                autoScroll
                  ? "border-cyan-500/30 bg-cyan-500/15 text-cyan-300"
                  : "border-white/[0.08] bg-cardin/80 text-ink4 hover:text-ink2 hover:bg-white/[0.05]"
              }`}
              title={t.logs.autoScroll}
            >
              <ArrowDown className={`h-3 w-3 ${autoScroll ? "text-cyan-400" : ""}`} />
              <span>{t.logs.autoScroll}</span>
            </button>

            {/* Clear Logs */}
            <button
              type="button"
              onClick={onClearLogs}
              className="flex h-8 items-center gap-1.5 rounded-panel border border-white/[0.08] bg-cardin/80 px-2.5 text-xs text-ink4 hover:bg-rose-500/15 hover:border-rose-500/30 hover:text-rose-300 transition-all cursor-pointer"
              title={t.logs.clear}
            >
              <Trash2 className="h-3.5 w-3.5" />
              <span>{t.logs.clear}</span>
            </button>
          </div>
        </div>

        {/* Terminal Display */}
        <div
          ref={logContainerRef}
          className={`${
            fullHeight ? "h-[calc(100vh-210px)] min-h-[460px]" : "h-72"
          } overflow-y-auto rounded-panel border border-white/[0.06] bg-cardin/95 p-3.5 font-mono text-[11px] leading-relaxed shadow-inner select-text`}
        >
          {displayLogs.length === 0 ? (
            <div className="flex flex-col h-full items-center justify-center text-center p-6 space-y-2">
              <div className="flex h-12 w-12 items-center justify-center rounded-card bg-white/[0.03] border border-white/[0.06] text-ink5">
                <Terminal className="h-6 w-6" />
              </div>
              <p className="text-xs font-semibold text-ink4">{t.logs.emptyTitle}</p>
              <p className="text-[11px] text-ink5 max-w-sm">
                {t.logs.emptyDesc}
              </p>
              {onRefreshLogs && (
                <button
                  type="button"
                  onClick={onRefreshLogs}
                  className="mt-2 flex items-center gap-1.5 rounded-control border border-cyan-500/30 bg-cyan-500/10 px-3 py-1 text-xs font-semibold text-cyan-300 hover:bg-cyan-500/20 transition-all cursor-pointer"
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                  <span>{t.logs.refreshNow}</span>
                </button>
              )}
            </div>
          ) : (
            displayLogs.map((log) => (
              <div
                key={log.id ?? `${log.timestamp}-${log.message}`}
                className="flex items-start gap-2.5 py-1 hover:bg-white/[0.03] rounded-chip px-1.5 transition-colors"
              >
                <span className="text-ink5 shrink-0 select-none font-mono">
                  {formatTimestamp(log.timestamp)}
                </span>
                <span
                  className={`rounded-chip px-1.5 py-0.2 text-[9px] uppercase font-bold shrink-0 ${getLevelStyle(
                    log.level
                  )}`}
                >
                  {log.level}
                </span>
                <span className="text-ink2 break-all select-text font-mono">
                  {log.message}
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    );
  }
);
