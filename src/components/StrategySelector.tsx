import React, { useState } from "react";
import { LuaStrategy, SupervisorState } from "../types";
import {
  Sliders,
  FileCode,
  Check,
  Sparkles,
  HelpCircle,
  Plus,
  Pencil,
  Trash2,
  Play,
  Square,
  Zap,
  Loader2,
  Layers,
  Cpu,
  Wrench,
} from "lucide-react";
import { useI18n } from "../i18n";
import { CustomProfileModal } from "./CustomProfileModal";
import { ConfigGuideModal } from "./ConfigGuideModal";

interface StrategySelectorProps {
  strategies: LuaStrategy[];
  selectedStrategyId: string;
  activeStrategyId: string | null;
  supervisorState: SupervisorState;
  onSelectStrategy: (id: string) => void;
  onStartStrategy: (id: string) => void;
  onStopStrategy: () => void;
  onSaveCustomProfile: (strategy: LuaStrategy) => Promise<void>;
  onDeleteCustomProfile: (id: string) => Promise<void>;
  disabled?: boolean;
}

type TabType = "all" | "flowseal" | "zapret2" | "custom";

export const StrategySelector: React.FC<StrategySelectorProps> = React.memo(
  ({
    strategies,
    selectedStrategyId,
    activeStrategyId,
    supervisorState,
    onSelectStrategy,
    onStartStrategy,
    onStopStrategy,
    onSaveCustomProfile,
    onDeleteCustomProfile,
    disabled = false,
  }) => {
    const { t } = useI18n();

    const [activeTab, setActiveTab] = useState<TabType>("flowseal");
    const [isGuideOpen, setIsGuideOpen] = useState(false);
    const [isCustomModalOpen, setIsCustomModalOpen] = useState(false);
    const [editingStrategy, setEditingStrategy] = useState<LuaStrategy | null>(null);

    const getFriendlyDescription = (strat: LuaStrategy) => {
      if (strat.id === "antidpi" || strat.filename.includes("antidpi")) {
        return t.strategies.friendlyAntidpi;
      }
      if (strat.id === "auto" || strat.filename.includes("auto")) {
        return t.strategies.friendlyAuto;
      }
      return strat.description || t.strategies.friendlyCustom;
    };

    const filteredStrategies = strategies.filter((strat) => {
      if (activeTab === "all") return true;
      if (activeTab === "flowseal") return strat.category === "flowseal" || strat.engine === "winws";
      if (activeTab === "zapret2") return strat.category === "zapret2" && strat.engine === "winws2";
      if (activeTab === "custom") return strat.category === "custom";
      return true;
    });

    const handleCreateCustom = () => {
      setEditingStrategy(null);
      setIsCustomModalOpen(true);
    };

    const handleEditCustom = (strat: LuaStrategy, e: React.MouseEvent) => {
      e.stopPropagation();
      setEditingStrategy(strat);
      setIsCustomModalOpen(true);
    };

    const handleDeleteCustom = async (id: string, e: React.MouseEvent) => {
      e.stopPropagation();
      if (window.confirm(t.strategies.deleteCustomConfirm)) {
        await onDeleteCustomProfile(id);
      }
    };

    return (
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        {/* Header with Title and Modals Triggers */}
        <div className="flex flex-wrap items-center justify-between gap-2.5 mb-3">
          <div className="flex items-center gap-2">
            <Sliders className="h-4 w-4 text-blue-400" />
            <h2 className="text-sm font-semibold text-white">{t.strategies.title}</h2>
            <span className="text-xs text-slate-400">
              ({strategies.length} {t.strategies.discovered})
            </span>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setIsGuideOpen(true)}
              className="flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-800/80 px-2.5 py-1 text-xs font-medium text-emerald-400 hover:bg-slate-700 hover:text-emerald-300 transition-colors cursor-pointer"
            >
              <HelpCircle className="h-3.5 w-3.5" />
              <span>{t.strategies.configGuideBtn}</span>
            </button>

            <button
              type="button"
              disabled={disabled}
              onClick={handleCreateCustom}
              className="flex items-center gap-1.5 rounded-lg border border-blue-500/40 bg-blue-500/10 px-2.5 py-1 text-xs font-semibold text-blue-300 hover:bg-blue-500/20 hover:text-white transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>{t.strategies.addCustomBtn}</span>
            </button>
          </div>
        </div>

        {/* Category Tabs */}
        <div className="flex items-center gap-1.5 border-b border-[#202534] pb-3 mb-4 overflow-x-auto text-xs">
          <button
            type="button"
            onClick={() => setActiveTab("flowseal")}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "flowseal"
                ? "bg-[#1d2332] text-white border border-[#384359] shadow-sm"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/[0.04]"
            }`}
          >
            <Sparkles className="h-3.5 w-3.5 text-blue-400" />
            <span>{t.strategies.tabFlowseal}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("zapret2")}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "zapret2"
                ? "bg-[#1d2332] text-white border border-[#384359] shadow-sm"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/[0.04]"
            }`}
          >
            <Cpu className="h-3.5 w-3.5 text-blue-400" />
            <span>{t.strategies.tabZapret2}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("custom")}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "custom"
                ? "bg-[#1d2332] text-white border border-[#384359] shadow-sm"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/[0.04]"
            }`}
          >
            <Wrench className="h-3.5 w-3.5 text-blue-400" />
            <span>{t.strategies.tabCustom}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("all")}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "all"
                ? "bg-[#1d2332] text-white border border-[#384359] shadow-sm"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/[0.04]"
            }`}
          >
            <Layers className="h-3.5 w-3.5 text-slate-400" />
            <span>{t.strategies.tabAll}</span>
          </button>
        </div>

        {/* Strategies Cards Grid */}
        {filteredStrategies.length === 0 ? (
          <div className="py-8 text-center text-xs text-slate-500">
            {t.strategies.noStrategiesInTab}
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2.5">
            {filteredStrategies.map((strat) => {
              const isSelected = selectedStrategyId === strat.id;
              const isActive = supervisorState === "running" && activeStrategyId === strat.id;
              const isTransitioning = supervisorState === "starting" || supervisorState === "stopping";
              const isStartingThis = supervisorState === "starting" && selectedStrategyId === strat.id;
              const isStoppingThis = supervisorState === "stopping" && activeStrategyId === strat.id;
              const isBypassRunning = supervisorState === "running";
              const friendlyDesc = getFriendlyDescription(strat);
              const isRecommended = strat.id === "flowseal-alt";
              const isUltimate = strat.id === "zapret2-ru-ultimate";
              const is4k = strat.id === "flowseal-alt11";
              const isFlowseal = strat.engine === "winws";
              const isCustom = strat.category === "custom";

              let cardStyle = "border-[#212635] bg-[#141720] text-slate-300 hover:border-[#333d52] hover:bg-[#181d28]";
              if (isActive) {
                cardStyle = "border-emerald-500/60 bg-[#13251e] text-white shadow-sm ring-1 ring-emerald-500/30";
              } else if (isSelected) {
                cardStyle = "border-blue-500/60 bg-[#151f33] text-white shadow-sm ring-1 ring-blue-500/30";
              }

              return (
                <div
                  key={strat.id}
                  onClick={() => onSelectStrategy(strat.id)}
                  onDoubleClick={() => {
                    if (isTransitioning) return;
                    if (isActive) {
                      onStopStrategy();
                    } else {
                      onStartStrategy(strat.id);
                    }
                  }}
                  className={`relative flex flex-col justify-between rounded-2xl border p-4 text-left transition-all duration-200 cursor-pointer min-h-[175px] ${cardStyle} ${
                    isTransitioning ? "opacity-75" : ""
                  }`}
                >
                  {/* Top Content: Title, Badges, Description, Filename */}
                  <div>
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-start gap-2.5 min-w-0">
                        <div
                          className={`mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border ${
                            isActive
                              ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-400"
                              : isSelected
                              ? "bg-cyan-500/15 border-cyan-500/30 text-cyan-400"
                              : isCustom
                              ? "bg-amber-500/15 border-amber-500/30 text-amber-400"
                              : "bg-white/[0.04] border-white/[0.08] text-slate-400"
                          }`}
                        >
                          <FileCode className="h-4 w-4" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 flex-wrap">
                            <span className="text-xs font-bold truncate text-white">
                              {strat.name}
                            </span>
                            {isUltimate && (
                              <span className="flex items-center gap-0.5 rounded-md bg-cyan-500/20 border border-cyan-500/40 px-1.5 py-0.2 text-[9px] font-bold text-cyan-300 shrink-0">
                                <Sparkles className="h-2.5 w-2.5" />
                                <span>{t.strategies.flagshipBadge}</span>
                              </span>
                            )}
                            {isRecommended && (
                              <span className="flex items-center gap-0.5 rounded-md bg-emerald-500/20 border border-emerald-500/40 px-1.5 py-0.2 text-[9px] font-bold text-emerald-300 shrink-0">
                                <Sparkles className="h-2.5 w-2.5" />
                                <span>{t.strategies.badgeRecommended}</span>
                              </span>
                            )}
                            {is4k && (
                              <span className="flex items-center gap-0.5 rounded-md bg-purple-500/20 border border-purple-500/40 px-1.5 py-0.2 text-[9px] font-bold text-purple-300 shrink-0">
                                <span>{t.strategies.badge4k}</span>
                              </span>
                            )}
                            <span
                              className={`rounded-md px-1.5 py-0.2 text-[9px] font-mono font-medium shrink-0 ${
                                isFlowseal
                                  ? "bg-indigo-500/15 border border-indigo-500/30 text-indigo-300"
                                  : "bg-sky-500/15 border border-sky-500/30 text-sky-300"
                              }`}
                            >
                              {isFlowseal ? t.strategies.engineFlowseal : t.strategies.engineZapret2}
                            </span>
                          </div>
                          <div className="text-[11px] text-slate-400 mt-1 line-clamp-2 leading-relaxed min-h-[32px]">
                            {friendlyDesc}
                          </div>
                          <div className="text-[10px] font-mono text-slate-500 mt-1 truncate">
                            {strat.filename}
                          </div>
                        </div>
                      </div>

                      {/* Status indicator on top right */}
                      <div className="flex items-center gap-1.5 shrink-0 ml-1">
                        {isActive && (
                          <span className="flex items-center gap-1.5 rounded-full bg-emerald-500/20 border border-emerald-500/50 px-2 py-0.5 text-[10px] font-bold text-emerald-300">
                            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400 animate-pulse"></span>
                            <span>{t.strategies.cardActive}</span>
                          </span>
                        )}
                        {!isActive && isSelected && (
                          <div className="flex h-4 w-4 items-center justify-center rounded-full bg-cyan-500 text-white">
                            <Check className="h-2.5 w-2.5" />
                          </div>
                        )}
                      </div>
                    </div>
                  </div>

                  {/* Bottom Action Footer */}
                  <div className="mt-3 pt-3 border-t border-white/[0.06] flex items-center justify-between gap-2">
                    {/* Left: Custom Actions or Strategy ID Badge */}
                    <div className="flex items-center gap-1 min-w-0">
                      {isCustom ? (
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            onClick={(e) => handleEditCustom(strat, e)}
                            disabled={isTransitioning}
                            className="p-1.5 rounded-lg text-slate-400 hover:text-cyan-300 hover:bg-white/[0.06] transition-colors cursor-pointer"
                            title={t.strategies.editCustom}
                          >
                            <Pencil className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={(e) => handleDeleteCustom(strat.id, e)}
                            disabled={isTransitioning}
                            className="p-1.5 rounded-lg text-slate-400 hover:text-rose-400 hover:bg-rose-500/10 transition-colors cursor-pointer"
                            title={t.strategies.deleteCustom}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      ) : (
                        <span className="text-[10px] text-slate-400 font-mono truncate max-w-[105px] bg-white/[0.03] border border-white/[0.06] px-1.5 py-0.5 rounded-md">
                          {strat.id}
                        </span>
                      )}
                    </div>

                    {/* Right: 1-Click Action Buttons */}
                    <div className="shrink-0">
                      {isActive ? (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onStopStrategy();
                          }}
                          disabled={isTransitioning}
                          className="flex items-center gap-1.5 rounded-xl bg-rose-500/15 border border-rose-500/35 hover:bg-rose-500/25 text-rose-300 px-3 py-1.5 text-xs font-bold transition-all active:scale-95 cursor-pointer disabled:opacity-50"
                          title={t.controls.stop}
                        >
                          {isStoppingThis ? (
                            <>
                              <Loader2 className="h-3 w-3 animate-spin" />
                              <span>{t.controls.stopping}</span>
                            </>
                          ) : (
                            <>
                              <Square className="h-3 w-3 fill-current" />
                              <span>{t.strategies.cardStop}</span>
                            </>
                          )}
                        </button>
                      ) : isBypassRunning ? (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onStartStrategy(strat.id);
                          }}
                          disabled={isTransitioning}
                          className="flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-indigo-600 to-cyan-600 hover:from-indigo-500 hover:to-cyan-500 text-white px-3 py-1.5 text-xs font-bold shadow-md shadow-indigo-600/20 transition-all active:scale-95 cursor-pointer disabled:opacity-50"
                          title={t.strategies.cardSwitch}
                        >
                          {isStartingThis ? (
                            <>
                              <Loader2 className="h-3 w-3 animate-spin" />
                              <span>{t.controls.starting}</span>
                            </>
                          ) : (
                            <>
                              <Zap className="h-3 w-3 fill-current" />
                              <span>{t.strategies.cardSwitch}</span>
                            </>
                          )}
                        </button>
                      ) : (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onStartStrategy(strat.id);
                          }}
                          disabled={isTransitioning}
                          className={`flex items-center gap-1.5 rounded-xl px-3 py-1.5 text-xs font-bold transition-all active:scale-95 cursor-pointer disabled:opacity-50 ${
                            isSelected
                              ? "bg-blue-600 hover:bg-blue-500 text-white shadow-md shadow-blue-600/25"
                              : "bg-white/[0.06] hover:bg-white/[0.1] text-slate-200 border border-white/[0.1]"
                          }`}
                          title={t.strategies.cardStart}
                        >
                          {isStartingThis ? (
                            <>
                              <Loader2 className="h-3 w-3 animate-spin" />
                              <span>{t.controls.starting}</span>
                            </>
                          ) : (
                            <>
                              <Play className="h-3 w-3 fill-current" />
                              <span>{t.strategies.cardStart}</span>
                            </>
                          )}
                        </button>
                      )}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}

        {/* Modals */}
        <CustomProfileModal
          isOpen={isCustomModalOpen}
          onClose={() => setIsCustomModalOpen(false)}
          onSave={onSaveCustomProfile}
          editingStrategy={editingStrategy}
        />

        <ConfigGuideModal
          isOpen={isGuideOpen}
          onClose={() => setIsGuideOpen(false)}
        />
      </div>
    );
  }
);
