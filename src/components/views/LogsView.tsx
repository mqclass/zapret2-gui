import React from "react";
import { LogViewer } from "../LogViewer";
import { LogEntry } from "../../types";

interface LogsViewProps {
  logs: LogEntry[];
  onClearLogs: () => void;
  onRefreshLogs?: () => void;
  isRefreshing?: boolean;
}

export const LogsView: React.FC<LogsViewProps> = React.memo(
  ({ logs, onClearLogs, onRefreshLogs, isRefreshing }) => {
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
