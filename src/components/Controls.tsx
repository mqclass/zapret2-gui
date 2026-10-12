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
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        <div className="flex flex-wrap items-center justify-between gap-4">
          {/* Main Controls Area */}
          <div className="flex items-center gap-2.5 flex-wrap">
            {/* Primary Start / Stop Button */}
            <button
              onClick={handleToggle}
              disabled={disabled || isTransitioning}
              className={`flex items-center gap-2.5 rounded-xl px-5 py-2.5 font-semibold text-sm shadow-lg transition-all ${
                isRunning
                  ? "bg-rose-600 hover:bg-rose-500 text-white shadow-rose-600/20 active:scale-95"
                  : "bg-blue-600 hover:bg-blue-500 text-white shadow-blue-600/20 active:scale-95"
              } ${disabled || isTransitioning ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}`}
            >
              {isStarting ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin text-white" />
                  <span>{t.controls.starting}</span>
                </>
              ) : isStopping ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin text-white" />
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
                      <span className="text-xs text-blue-200 font-normal">
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
                className="flex items-center gap-2 rounded-xl bg-indigo-600 hover:bg-indigo-500 px-4 py-2.5 font-semibold text-xs text-white shadow-md shadow-indigo-600/20 transition-all active:scale-95 cursor-pointer disabled:opacity-50"
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
              className={`flex items-center gap-1.5 rounded-xl border border-slate-800 px-3 py-2 text-xs font-medium transition-all cursor-pointer ${
                showAdvanced
                  ? "bg-slate-800 text-white border-slate-700"
                  : "bg-slate-900/60 text-slate-400 hover:text-slate-200"
              }`}
            >
              <Settings2 className="h-3.5 w-3.5" />
              <span>{t.controls.advanced}</span>
            </button>
          </div>

          <div className="text-xs text-slate-400 flex items-center gap-2">
            <Shield className="h-3.5 w-3.5 text-blue-400 shrink-0" />
            <span className="text-slate-400">{t.controls.isolationActive}</span>
          </div>
        </div>

        {/* Advanced Custom Arguments Drawer */}
        {showAdvanced && (
          <div className="mt-4 border-t border-slate-800/80 pt-3">
            <div className="flex items-center gap-1.5 mb-2 text-xs font-medium text-slate-300">
              <Terminal className="h-3.5 w-3.5 text-blue-400" />
              <span>{t.controls.customArgsLabel}</span>
            </div>
            <input
              type="text"
              value={customArgsInput}
              onChange={(e) => setCustomArgsInput(e.target.value)}
              disabled={isRunning}
              placeholder={t.controls.customArgsPlaceholder}
              className="w-full rounded-lg border border-slate-800 bg-slate-950 px-3 py-2 text-xs font-mono text-slate-200 placeholder-slate-600 focus:border-blue-500 focus:outline-none transition-all disabled:opacity-50"
            />
            <p className="mt-1.5 text-[11px] text-slate-500">
              {t.controls.customArgsHint}
            </p>
          </div>
        )}
      </div>
    );
  }
);
