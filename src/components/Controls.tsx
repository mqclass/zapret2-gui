import React, { useState } from "react";
import { Play, Square, Settings2, Terminal, Shield, Zap, Loader2 } from "lucide-react";
import { SupervisorState } from "../types";
import { useI18n } from "../i18n";

interface ControlsProps {
  state: SupervisorState;
  activeStrategyName?: string | null;
  selectedStrategyName?: string | null;
  onStart: (customArgs?: string[]) => void;
  onStop: () => void;
  onSwitchToSelected?: () => void;
  disabled?: boolean;
}

export const Controls: React.FC<ControlsProps> = React.memo(
  ({
    state,
    activeStrategyName,
    selectedStrategyName,
    onStart,
    onStop,
    onSwitchToSelected,
    disabled = false,
  }) => {
    const { t } = useI18n();
    const [showAdvanced, setShowAdvanced] = useState(false);
    const [customArgsInput, setCustomArgsInput] = useState("");

    const isRunning = state === "running";
    const isStarting = state === "starting";
    const isStopping = state === "stopping";
    const isTransitioning = isStarting || isStopping;

    const hasDifferentSelection =
      isRunning &&
      selectedStrategyName &&
      activeStrategyName &&
      selectedStrategyName !== activeStrategyName;

    const handleToggle = () => {
      if (isRunning) {
        onStop();
      } else {
        const raw = customArgsInput.trim();
        let parsedArgs: string[] | undefined = undefined;
        if (raw) {
          const tokens: string[] = [];
          let current = "";
          let inQuotes = false;
          let quoteChar = "";

          for (let i = 0; i < raw.length; i++) {
            const ch = raw[i];
            if ((ch === '"' || ch === "'") && (!inQuotes || quoteChar === ch)) {
              inQuotes = !inQuotes;
              quoteChar = inQuotes ? ch : "";
              current += ch;
            } else if (/\s/.test(ch) && !inQuotes) {
              if (current.trim().length > 0) {
                tokens.push(current.trim());
                current = "";
              }
            } else {
              current += ch;
            }
          }
          if (current.trim().length > 0) {
            tokens.push(current.trim());
          }
          parsedArgs = tokens.length > 0 ? tokens : undefined;
        }
        onStart(parsedArgs);
      }
    };

    return (
      <div className="rounded-panel border border-raise bg-card/40 p-4">
        <div className="flex flex-wrap items-center justify-between gap-4">
          {/* Main Controls Area */}
          <div className="flex items-center gap-2.5 flex-wrap">
            {/* Primary Start / Stop Button */}
            <button
              onClick={handleToggle}
              disabled={disabled || isTransitioning}
              className={`flex items-center gap-2.5 rounded-panel px-5 py-2.5 font-semibold text-sm shadow-lg transition-all ${
                isRunning
                  ? "bg-rose-600 hover:bg-rose-500 text-ink shadow-rose-600/20 active:scale-95"
                  : "bg-accent-fill hover:bg-accent text-accent-contrast shadow-accent/20 active:scale-95"
              } ${disabled || isTransitioning ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}`}
            >
              {isStarting ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin text-ink" />
                  <span>{t.controls.starting}</span>
                </>
              ) : isStopping ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin text-ink" />
                  <span>{t.controls.stopping}</span>
                </>
              ) : isRunning ? (
                <>
                  <Square className="h-4 w-4 fill-white" />
                  <div className="flex items-center gap-1.5">
                    <span>{t.controls.stop}</span>
                    {activeStrategyName && (
                      <span className="text-xs text-rose-200 font-normal">
                        ({activeStrategyName})
                      </span>
                    )}
                  </div>
                </>
              ) : (
                <>
                  <Play className="h-4 w-4 fill-white" />
                  <div className="flex items-center gap-1.5">
                    <span>{t.controls.start}</span>
                    {selectedStrategyName && (
                      <span className="text-xs text-accent-soft font-normal">
                        ({selectedStrategyName})
                      </span>
                    )}
                  </div>
                </>
              )}
            </button>

            {/* Quick Switch Button if user selected another config while running */}
            {hasDifferentSelection && onSwitchToSelected && (
              <button
                onClick={onSwitchToSelected}
                disabled={disabled || isTransitioning}
                className="flex items-center gap-2 rounded-panel bg-indigo-600 hover:bg-indigo-500 px-4 py-2.5 font-semibold text-xs text-ink shadow-md shadow-accent-fill/20 transition-all active:scale-95 cursor-pointer disabled:opacity-50"
                title={`Переключить обход на "${selectedStrategyName}"`}
              >
                <Zap className="h-3.5 w-3.5 text-amber-300 fill-amber-300" />
                <span>
                  {t.strategies.cardSwitch}: {selectedStrategyName}
                </span>
              </button>
            )}

            {/* Advanced Parameters toggle */}
            <button
              onClick={() => setShowAdvanced(!showAdvanced)}
              className={`flex items-center gap-1.5 rounded-panel border border-raise px-3 py-2 text-xs font-medium transition-all cursor-pointer ${
                showAdvanced
                  ? "bg-raise text-ink border-linestrong"
                  : "bg-card/60 text-ink4 hover:text-ink2"
              }`}
            >
              <Settings2 className="h-3.5 w-3.5" />
              <span>{t.controls.advanced}</span>
            </button>
          </div>

          <div className="text-xs text-ink4 flex items-center gap-2">
            <Shield className="h-3.5 w-3.5 text-accent shrink-0" />
            <span className="text-ink4">{t.controls.isolationActive}</span>
          </div>
        </div>

        {/* Advanced Custom Arguments Drawer */}
        {showAdvanced && (
          <div className="mt-4 border-t border-raise/80 pt-3">
            <div className="flex items-center gap-1.5 mb-2 text-xs font-medium text-ink3">
              <Terminal className="h-3.5 w-3.5 text-accent" />
              <span>{t.controls.customArgsLabel}</span>
            </div>
            <input
              type="text"
              value={customArgsInput}
              onChange={(e) => setCustomArgsInput(e.target.value)}
              disabled={isRunning}
              placeholder={t.controls.customArgsPlaceholder}
              className="w-full rounded-control border border-raise bg-cardin px-3 py-2 text-xs font-mono text-ink2 placeholder-ink5 focus:border-accent-line focus:outline-none transition-all disabled:opacity-50"
            />
            <p className="mt-1.5 text-[11px] text-ink5">
              {t.controls.customArgsHint}
            </p>
          </div>
        )}
      </div>
    );
  }
);
