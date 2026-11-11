import React from "react";
import { DiagnosticsReport, LogEntry, LuaStrategy } from "../../types";
import { NavTab } from "../Sidebar";
import { DiagnosticsView } from "./DiagnosticsView";
import { LogsView } from "./LogsView";

interface DiagnosticsLogsViewProps {
  /** Shell view mode (redesign): false = Простой вид, true = Эксперт. */
  expertMode?: boolean;
  diagnostics: DiagnosticsReport;
  onRefreshProbes: () => void;
  isRefreshingProbes: boolean;
  onSelectAndStartStrategy: (id: string) => void;
  onNavigateTab: (tab: NavTab) => void;
  logs: LogEntry[];
  onClearLogs: () => void;
  onRefreshLogs?: () => void;
  isRefreshingLogs?: boolean;
  /** Strategy catalog for human names in the simple digest. */
  strategies?: LuaStrategy[];
  /** Switch the shell to expert mode (the log digest's "show full log"). */
  onExpertModeChange: (expert: boolean) => void;
}

/**
 * Redesign stage 5: the unified "Диагностика и Логи" tab.
 * Simple view: compact probe cards, the classifier card with its
 * target buttons, a one-line doctor row, and a digest of the last
 * events with a "copy diagnostics" button. Fits 1050x760 without
 * scrolling.
 * Expert view: the full diagnostics view (doctor banner, classifier
 * with the custom-domain form, detailed probe cards, custom ping)
 * and the full log terminal with filters, stacked exactly as before
 * the redesign.
 */
export const DiagnosticsLogsView: React.FC<DiagnosticsLogsViewProps> = React.memo(
  ({
    expertMode = false,
    diagnostics,
    onRefreshProbes,
    isRefreshingProbes,
    onSelectAndStartStrategy,
    onNavigateTab,
    logs,
    onClearLogs,
    onRefreshLogs,
    isRefreshingLogs,
    strategies,
    onExpertModeChange,
  }) => {
    if (!expertMode) {
      return (
        <div className="space-y-3 pb-3">
          <DiagnosticsView
            variant="simple"
            diagnostics={diagnostics}
            onRefreshProbes={onRefreshProbes}
            isRefreshingProbes={isRefreshingProbes}
            onSelectAndStartStrategy={onSelectAndStartStrategy}
            onNavigateTab={onNavigateTab}
          />
          <LogsView
            variant="simple"
            logs={logs}
            onClearLogs={onClearLogs}
            onRefreshLogs={onRefreshLogs}
            isRefreshing={isRefreshingLogs}
            onShowExpert={() => onExpertModeChange(true)}
            strategies={strategies}
          />
        </div>
      );
    }

    return (
      <div className="space-y-6">
        <DiagnosticsView
          variant="full"
          diagnostics={diagnostics}
          onRefreshProbes={onRefreshProbes}
          isRefreshingProbes={isRefreshingProbes}
          onSelectAndStartStrategy={onSelectAndStartStrategy}
          onNavigateTab={onNavigateTab}
        />
        <LogsView
          variant="full"
          logs={logs}
          onClearLogs={onClearLogs}
          onRefreshLogs={onRefreshLogs}
          isRefreshing={isRefreshingLogs}
        />
      </div>
    );
  }
);
