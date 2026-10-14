import React from "react";
import { StrategySelector } from "../StrategySelector";
import { Controls } from "../Controls";
import { LuaStrategy, SupervisorState } from "../../types";

interface StrategiesViewProps {
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
}

export const StrategiesView: React.FC<StrategiesViewProps> = React.memo(
  ({
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
  }) => {
    const activeStratObj = strategies.find((s) => s.id === activeStrategyId);
    const selectedStratObj = strategies.find((s) => s.id === selectedStrategyId);
    const activeStratName = activeStratObj?.name || activeStrategyId;
    const selectedStratName = selectedStratObj?.name || selectedStrategyId;

    return (
      <div className="space-y-6 pb-6">
        {/* Strategy Selector with Categories and Custom Profiles */}
        <StrategySelector
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
      </div>
    );
  }
);
