import React, { useState, useEffect, useRef, useMemo } from "react";
import { LogEntry } from "../types";
import { Terminal, Trash2, ArrowDown, Search } from "lucide-react";
import { useI18n } from "../i18n";

interface LogViewerProps {
  logs: LogEntry[];
  onClearLogs: () => void;
}

export const LogViewer: React.FC<LogViewerProps> = React.memo(({ logs, onClearLogs }) => {
  const { t } = useI18n();
  const [filterLevel, setFilterLevel] = useState<string>("all");
  const [searchTerm, setSearchTerm] = useState<string>("");
  const [autoScroll, setAutoScroll] = useState<boolean>(true);
  const logContainerRef = useRef<HTMLDivElement>(null);

  // PERFORMANCE OPTIMIZATION: Memoize filtered and sliced log entries
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
      displayLogs: filtered.slice(-100),
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
        return "text-rose-400 bg-rose-500/10";
      case "warn":
        return "text-amber-400 bg-amber-500/10";
      case "info":
        return "text-blue-400 bg-blue-500/10";
      case "stdout":
      default:
        return "text-emerald-400 bg-emerald-500/10";
    }
  };

  return (
    <div className="flex flex-col rounded-xl border border-slate-800 bg-slate-900/40 p-4">
      {/* Header & Controls */}
      <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
        <div className="flex items-center gap-2">
          <Terminal className="h-4 w-4 text-emerald-400" />
          <h2 className="text-sm font-semibold text-white">{t.logs.title}</h2>
          <span className="rounded-md bg-slate-800 px-2 py-0.5 text-[11px] font-mono text-slate-400">
            {logs.length} {t.logs.lines}
          </span>
          {filteredCount > 100 && (
            <span className="text-[10px] text-slate-500">
              ({t.logs.showingLatest})
            </span>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* Search Box */}
          <div className="relative">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
            <input
              type="text"
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              placeholder={t.logs.searchPlaceholder}
              className="h-7 w-36 rounded-lg border border-slate-800 bg-slate-950 pl-8 pr-2 text-xs text-slate-200 placeholder-slate-600 focus:border-blue-500 focus:outline-none"
            />
          </div>

          {/* Level Filter */}
          <select
            value={filterLevel}
            onChange={(e) => setFilterLevel(e.target.value)}
            className="h-7 rounded-lg border border-slate-800 bg-slate-950 px-2 text-xs text-slate-300 focus:border-blue-500 focus:outline-none"
          >
            <option value="all">{t.logs.allLevels}</option>
            <option value="stdout">stdout</option>
            <option value="stderr">stderr</option>
            <option value="info">info</option>
            <option value="warn">warn</option>
            <option value="error">error</option>
          </select>

          {/* Auto Scroll toggle */}
          <button
            onClick={() => setAutoScroll(!autoScroll)}
            className={`flex h-7 items-center gap-1 rounded-lg border px-2 text-xs transition-all cursor-pointer ${
              autoScroll
                ? "border-blue-500/30 bg-blue-500/15 text-blue-400"
                : "border-slate-800 bg-slate-950 text-slate-400 hover:text-slate-200"
            }`}
            title={t.logs.autoScroll}
          >
            <ArrowDown className={`h-3 w-3 ${autoScroll ? "text-blue-400" : ""}`} />
            <span>{t.logs.autoScroll}</span>
          </button>

          {/* Clear Logs */}
          <button
            onClick={onClearLogs}
            className="flex h-7 items-center gap-1 rounded-lg border border-slate-800 bg-slate-950 px-2 text-xs text-slate-400 hover:bg-slate-800 hover:text-white transition-all cursor-pointer"
            title={t.logs.clear}
          >
            <Trash2 className="h-3 w-3" />
            <span>{t.logs.clear}</span>
          </button>
        </div>
      </div>

      {/* Terminal Display */}
      <div
        ref={logContainerRef}
        className="h-64 overflow-y-auto rounded-lg border border-slate-950 bg-slate-950/90 p-3 font-mono text-[11px] leading-relaxed shadow-inner"
      >
        {displayLogs.length === 0 ? (
          <div className="flex h-full items-center justify-center text-slate-600">
            {t.logs.empty}
          </div>
        ) : (
          displayLogs.map((log, index) => (
            <div
              key={index}
              className="flex items-start gap-2 py-0.5 hover:bg-slate-900/60 rounded px-1"
            >
              <span className="text-slate-600 shrink-0 select-none">
                {formatTimestamp(log.timestamp)}
              </span>
              <span
                className={`rounded px-1.5 py-0 text-[10px] uppercase font-bold shrink-0 ${getLevelStyle(
                  log.level
                )}`}
              >
                {log.level}
              </span>
              <span className="text-slate-300 break-all select-text">{log.message}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
});
