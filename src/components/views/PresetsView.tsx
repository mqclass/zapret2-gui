import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ListFilter, Pencil } from "lucide-react";
import { LuaStrategy, SplitTunnelMode, SupervisorState } from "../../types";
import { useI18n } from "../../i18n";
import { StrategySelector } from "../StrategySelector";
import { Controls } from "../Controls";
import { ListsView } from "./ListsView";
import { TunnelModeSection } from "../TunnelModeSection";

interface PresetsViewProps {
  /** Shell view mode (redesign): false = Простой вид, true = Эксперт. */
  expertMode?: boolean;
  strategies: LuaStrategy[];
  selectedStrategyId: string;
  activeStrategyId: string | null;
  supervisorState: SupervisorState;
  onSelectStrategy: (id: string) => void;
  onStartStrategy: (id: string) => void;
  onStopStrategy: () => void;
  onStart: (customArgs?: string[]) => void;
  onSaveCustomProfile: (strategy: LuaStrategy) => Promise<void>;
  onDeleteCustomProfile: (id: string) => Promise<void>;
  disabled?: boolean;
  // Lua tunnel modes (expert section).
  splitTunnelMode: SplitTunnelMode;
  appliedMode?: SplitTunnelMode | null;
  activeCustomArgs?: boolean;
  onModeChange: (mode: SplitTunnelMode, opts?: { silent?: boolean }) => Promise<void>;
  // Auto-hostlist (simple summary card).
  autoHostlistOn: boolean;
  onToggleAutoHostlist: () => void;
  /** Switch the shell to expert mode (summary card's "edit lists" action). */
  onExpertModeChange: (expert: boolean) => void;
}

/**
 * Simple-view summary of the domain lists: the auto-hostlist switch
 * and entry counts in plain words. The full editor lives in the
 * expert section below (ListsView). Counts are best-effort: a missing
 * or unreadable file shows a dash instead of a number.
 */
const HostlistSummary: React.FC<{
  autoHostlistOn: boolean;
  onToggleAutoHostlist: () => void;
  onEditLists: () => void;
  disabled?: boolean;
}> = ({ autoHostlistOn, onToggleAutoHostlist, onEditLists, disabled = false }) => {
  const { t } = useI18n();
  const [counts, setCounts] = useState<{ general: number | null; auto: number | null }>({
    general: null,
    auto: null,
  });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const next: { general: number | null; auto: number | null } = {
        general: null,
        auto: null,
      };
      for (const [key, file] of [
        ["general", "list-general.txt"],
        ["auto", "list-auto.txt"],
      ] as const) {
        try {
          const list = (await invoke<string[]>("read_domain_list", { filename: file })) ?? [];
          next[key] = list.length;
        } catch {
          // Keep null: the file may not exist yet (list-auto.txt is
          // created on first use) — the summary shows a dash.
        }
      }
      if (!cancelled) setCounts(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [autoHostlistOn]);

  const fmt = (n: number | null) => (n === null ? "—" : String(n));

  return (
    <div className="t-card p-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-baseline gap-2 min-w-0">
          <ListFilter className="h-4 w-4 text-accent shrink-0 self-center" />
          <div className="text-sm font-semibold text-ink shrink-0">
            {t.presetsTab.listsTitle}
          </div>
          {/* One line: the toggle button on the right already states
              the on/off action, so the state lives here as one word. */}
          <div className="text-[11px] text-ink4 truncate">
            {t.settingsTab.autoHostlistTitle} —{" "}
            {autoHostlistOn ? t.settingsTab.autoHostlistOn : t.settingsTab.autoHostlistOff}
            {" · "}
            {t.presetsTab.listsGeneral}: {fmt(counts.general)}
            {" · "}
            {t.presetsTab.listsAuto}: {fmt(counts.auto)}
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <button
            type="button"
            onClick={onEditLists}
            className="flex items-center gap-1.5 rounded-control border border-line bg-raise hover:bg-hovert px-2.5 py-1.5 text-xs font-medium text-ink3 transition-colors cursor-pointer"
          >
            <Pencil className="h-3.5 w-3.5" />
            <span>{t.presetsTab.listsEdit}</span>
          </button>
          <button
            type="button"
            onClick={onToggleAutoHostlist}
            disabled={disabled}
            className="rounded-control bg-accent-fill hover:bg-accent text-ink px-3 py-1.5 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50"
          >
            {autoHostlistOn
              ? t.settingsTab.autoHostlistBtnDisable
              : t.settingsTab.autoHostlistBtnEnable}
          </button>
        </div>
      </div>
    </div>
  );
};

/**
 * Redesign stage 3: the unified "Пресеты и Хосты" tab.
 * Simple view: wizard + compact strategy list + hostlist summary.
 * Expert view: full strategy grid + start/stop controls + Lua tunnel
 * modes + the full domain-list editor.
 */
export const PresetsView: React.FC<PresetsViewProps> = React.memo(
  ({
    expertMode = false,
    strategies,
    selectedStrategyId,
    activeStrategyId,
    supervisorState,
    onSelectStrategy,
    onStartStrategy,
    onStopStrategy,
    onStart,
    onSaveCustomProfile,
    onDeleteCustomProfile,
    disabled = false,
    splitTunnelMode,
    appliedMode = null,
    activeCustomArgs = false,
    onModeChange,
    autoHostlistOn,
    onToggleAutoHostlist,
    onExpertModeChange,
  }) => {
    const activeStratObj = strategies.find((s) => s.id === activeStrategyId);
    const selectedStratObj = strategies.find((s) => s.id === selectedStrategyId);
    const activeStratName = activeStratObj?.name || activeStrategyId;
    const selectedStratName = selectedStratObj?.name || selectedStrategyId;

    if (!expertMode) {
      return (
        <div className="space-y-3 pb-3">
          <StrategySelector
            variant="simple"
            strategies={strategies}
            selectedStrategyId={selectedStrategyId}
            activeStrategyId={activeStrategyId}
            supervisorState={supervisorState}
            onSelectStrategy={onSelectStrategy}
            onStartStrategy={onStartStrategy}
            onStopStrategy={onStopStrategy}
            onSaveCustomProfile={onSaveCustomProfile}
            onDeleteCustomProfile={onDeleteCustomProfile}
            disabled={disabled}
            onShowAllStrategies={() => onExpertModeChange(true)}
          />
          <HostlistSummary
            autoHostlistOn={autoHostlistOn}
            onToggleAutoHostlist={onToggleAutoHostlist}
            onEditLists={() => onExpertModeChange(true)}
            disabled={disabled}
          />
        </div>
      );
    }

    return (
      <div className="space-y-6 pb-6">
        <StrategySelector
          variant="full"
          strategies={strategies}
          selectedStrategyId={selectedStrategyId}
          activeStrategyId={activeStrategyId}
          supervisorState={supervisorState}
          onSelectStrategy={onSelectStrategy}
          onStartStrategy={onStartStrategy}
          onStopStrategy={onStopStrategy}
          onSaveCustomProfile={onSaveCustomProfile}
          onDeleteCustomProfile={onDeleteCustomProfile}
          disabled={disabled}
        />

        {/* Custom Arguments & Quick Action Controls */}
        <Controls
          state={supervisorState}
          activeStrategyName={activeStratName}
          selectedStrategyName={selectedStratName}
          onStart={onStart}
          onStop={onStopStrategy}
          onSwitchToSelected={() => onStartStrategy(selectedStrategyId)}
          disabled={disabled}
        />

        {/* Lua tunnel modes (FULL / STRICT / GAME) */}
        <TunnelModeSection
          currentMode={splitTunnelMode}
          appliedMode={appliedMode}
          isRunning={supervisorState === "running"}
          activeStrategyId={activeStrategyId}
          activeCustomArgs={activeCustomArgs}
          onModeChange={onModeChange}
          disabled={disabled}
        />

        {/* Domain lists: full editor */}
        <ListsView expertMode={expertMode} />
      </div>
    );
  }
);
