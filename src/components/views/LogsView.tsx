import React from "react";
import { LogViewer } from "../LogViewer";
import { LogEntry } from "../../types";

interface LogsViewProps {
  logs: LogEntry[];
  onClearLogs: () => void;
}

export const LogsView: React.FC<LogsViewProps> = React.memo(({ logs, onClearLogs }) => {
  return (
    <div className="space-y-4 pb-6">
      <LogViewer logs={logs} onClearLogs={onClearLogs} />
    </div>
  );
});
