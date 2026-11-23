import React from "react";
import { LogEntry } from "../../types";
import { DnsSpeedView } from "./DnsSpeedView";
import { TelegramView } from "./TelegramView";

interface DnsViewProps {
  /** Shell view mode (redesign): false = Простой вид, true = Эксперт. */
  expertMode?: boolean;
  logs: LogEntry[];
  /** Switch the shell to expert mode (simple cards' "more" actions). */
  onExpertModeChange: (expert: boolean) => void;
}

/**
 * Redesign stage 4: the unified "Ускорение и DNS" tab.
 * Simple view: a two-column grid of compact cards — DNS providers
 * (full width), smart hosts + YouTube throttle, speedtest + the
 * Telegram bridge card. Fits 1050x760 without scrolling.
 * Expert view: the full DNS/speed view and the full Telegram view,
 * stacked exactly as before the redesign.
 */
export const DnsView: React.FC<DnsViewProps> = React.memo(
  ({ expertMode = false, logs, onExpertModeChange }) => {
    if (!expertMode) {
      return (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-3 pb-3">
          <DnsSpeedView
            variant="simple"
            onShowExpert={() => onExpertModeChange(true)}
          />
          <TelegramView
            variant="simple"
            logs={logs}
            onShowExpert={() => onExpertModeChange(true)}
          />
        </div>
      );
    }

    return (
      <div className="space-y-6 pb-6">
        <DnsSpeedView variant="full" />
        <TelegramView variant="full" logs={logs} />
      </div>
    );
  }
);
