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
  Youtube,
  MessageCircle,
  Globe2,
  Settings2,
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
      <div className="space-y-4">
        {/* Goal picker: plain-language entry point for ordinary users */}
        <div className="t-card p-5">
          <div className="mb-3">
            <h2 className="text-sm font-semibold text-ink">{t.strategies.goalTitle}</h2>
            <p className="text-xs text-ink4 mt-0.5">{t.strategies.goalSubtitle}</p>
          </div>
          <div className="grid grid-cols-2 lg:grid-cols-4 gap-2.5">
            {(
              [
                {
                  key: "youtube",
                  icon: Youtube,
                  title: t.strategies.goalYoutube,
                  desc: t.strategies.goalYoutubeDesc,
                  presetId: "flowseal-alt",
                  tint: "text-red-400",
                },
                {
                  key: "discord",
                  icon: MessageCircle,
                  title: t.strategies.goalDiscord,
                  desc: t.strategies.goalDiscordDesc,
                  presetId: "flowseal-fake-tls-auto",
                  tint: "text-indigo-400",
                },
                {
                  key: "all",
                  icon: Globe2,
                  title: t.strategies.goalAll,
                  desc: t.strategies.goalAllDesc,
                  presetId: "flowseal-alt11",
                  tint: "text-emerald-400",
                },
                {
                  key: "custom",
                  icon: Settings2,
                  title: t.strategies.goalCustom,
                  desc: t.strategies.goalCustomDesc,
                  presetId: null as string | null,
                  tint: "text-amber-400",
                },
              ] as const
            ).map((g) => {
              const isCurrent =
                g.presetId !== null && selectedStrategyId === g.presetId;
              const presetMissing =
                g.presetId !== null &&
                !strategies.some((s) => s.id === g.presetId);
              return (
                <button
                  key={g.key}
                  type="button"
                  disabled={disabled || presetMissing}
                  onClick={() => {
                    if (g.presetId) {
                      onSelectStrategy(g.presetId);
                      onStartStrategy(g.presetId);
                    } else {
                      setIsCustomModalOpen(true);
                    }
                  }}
                  className={`group flex flex-col gap-2 rounded-panel border p-3.5 text-left transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-60 ${
                    isCurrent
                      ? "border-accent-line bg-accent-wash shadow-glow"
                      : "border-line bg-cardin hover:border-linestrong hover:bg-raise"
                  }`}
                >
                  <g.icon className={`h-5 w-5 ${g.tint}`} />
                  <div>
                    <div className="text-xs font-semibold text-ink flex items-center gap-1.5">
                      {g.title}
                      {isCurrent && (
                        <span className="t-badge t-badge-accent">
                          <Check className="h-3 w-3" />
                        </span>
                      )}
                    </div>
                    <p className="text-[11px] text-ink4 mt-0.5 leading-snug">{g.desc}</p>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      <div className="t-card p-4">
        {/* Header with Title and Modals Triggers */}
        <div className="flex flex-wrap items-center justify-between gap-2.5 mb-3">
          <div className="flex items-center gap-2">
            <Sliders className="h-4 w-4 text-accent" />
            <h2 className="text-sm font-semibold text-ink">{t.strategies.title}</h2>
            <span className="text-xs text-ink4">
              ({strategies.length} {t.strategies.discovered})
            </span>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setIsGuideOpen(true)}
              className="flex items-center gap-1.5 rounded-control border border-linestrong bg-raise/80 px-2.5 py-1 text-xs font-medium text-emerald-400 hover:bg-linestrong hover:text-emerald-300 transition-colors cursor-pointer"
            >
              <HelpCircle className="h-3.5 w-3.5" />
              <span>{t.strategies.configGuideBtn}</span>
            </button>

            <button
              type="button"
              disabled={disabled}
              onClick={handleCreateCustom}
              className="flex items-center gap-1.5 rounded-control border border-accent-line bg-accent-wash px-2.5 py-1 text-xs font-semibold text-accent-soft hover:bg-accent-wash hover:text-ink transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>{t.strategies.addCustomBtn}</span>
            </button>
          </div>
        </div>

        {/* Category Tabs */}
        <div className="flex items-center gap-1.5 border-b border-line pb-3 mb-4 overflow-x-auto text-xs">
          <button
            type="button"
            onClick={() => setActiveTab("flowseal")}
            className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "flowseal"
                ? "bg-hovert text-ink border border-linestrong shadow-sm"
                : "text-ink4 hover:text-ink2 hover:bg-raise"
            }`}
          >
            <Sparkles className="h-3.5 w-3.5 text-accent" />
            <span>{t.strategies.tabFlowseal}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("zapret2")}
            className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "zapret2"
                ? "bg-hovert text-ink border border-linestrong shadow-sm"
                : "text-ink4 hover:text-ink2 hover:bg-raise"
            }`}
          >
            <Cpu className="h-3.5 w-3.5 text-accent" />
            <span>{t.strategies.tabZapret2}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("custom")}
            className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "custom"
                ? "bg-hovert text-ink border border-linestrong shadow-sm"
                : "text-ink4 hover:text-ink2 hover:bg-raise"
            }`}
          >
            <Wrench className="h-3.5 w-3.5 text-accent" />
            <span>{t.strategies.tabCustom}</span>
          </button>
          <button
            type="button"
            onClick={() => setActiveTab("all")}
            className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 font-medium transition-all cursor-pointer shrink-0 ${
              activeTab === "all"
                ? "bg-hovert text-ink border border-linestrong shadow-sm"
                : "text-ink4 hover:text-ink2 hover:bg-raise"
            }`}
          >
            <Layers className="h-3.5 w-3.5 text-ink4" />
            <span>{t.strategies.tabAll}</span>
          </button>
        </div>

        {/* Strategies Cards Grid */}
        {filteredStrategies.length === 0 ? (
          <div className="py-8 text-center text-xs text-ink5">
            {t.strategies.noStrategiesInTab}
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2.5">
            {filteredStrategies.map((strat, idx) => {
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

              let cardStyle = "border-line bg-card text-ink3 hover:border-linestrong hover:bg-hovert";
              if (isActive) {
                cardStyle = "border-emerald-500/60 bg-okwash text-ink shadow-sm ring-1 ring-emerald-500/30";
              } else if (isSelected) {
                cardStyle = "border-accent-line bg-accent-wash text-ink shadow-sm ring-1 ring-accent-line";
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
                  className={`relative flex flex-col justify-between rounded-card border p-4 text-left transition-all duration-200 cursor-pointer min-h-[175px] stagger-item hover-lift ${cardStyle} ${
                    isTransitioning ? "opacity-75" : ""
                  }`}
                  style={{ "--stagger-i": Math.min(idx, 11) } as React.CSSProperties}
                >
                  {/* Top Content: Title, Badges, Description, Filename */}
                  <div>
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-start gap-2.5 min-w-0">
                        <div
                          className={`mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-control border ${
                            isActive
                              ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-400"
                              : isSelected
                              ? "bg-violet-500/15 border-violet-500/30 text-violet-400"
                              : isCustom
                              ? "bg-amber-500/15 border-amber-500/30 text-amber-400"
                              : "bg-raise border-line text-ink4"
                          }`}
                        >
                          <FileCode className="h-4 w-4" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 flex-wrap">
                            <span className="text-xs font-bold truncate text-ink">
                              {strat.name}
                            </span>
                            {isUltimate && (
                              <span className="flex items-center gap-0.5 rounded-chip bg-violet-500/20 border border-violet-500/40 px-1.5 py-0.5 text-[9px] font-bold text-violet-300 shrink-0">
                                <Sparkles className="h-2.5 w-2.5" />
                                <span>{t.strategies.flagshipBadge}</span>
                              </span>
                            )}
                            {isRecommended && (
                              <span className="flex items-center gap-0.5 rounded-chip bg-emerald-500/20 border border-emerald-500/40 px-1.5 py-0.5 text-[9px] font-bold text-emerald-300 shrink-0">
                                <Sparkles className="h-2.5 w-2.5" />
                                <span>{t.strategies.badgeRecommended}</span>
                              </span>
                            )}
                            {is4k && (
                              <span className="flex items-center gap-0.5 rounded-chip bg-purple-500/20 border border-purple-500/40 px-1.5 py-0.5 text-[9px] font-bold text-purple-300 shrink-0">
                                <span>{t.strategies.badge4k}</span>
                              </span>
                            )}
                            <span
                              className={`rounded-chip px-1.5 py-0.5 text-[9px] font-mono font-medium shrink-0 ${
                                isFlowseal
                                  ? "bg-indigo-500/15 border border-indigo-500/30 text-indigo-300"
                                  : "bg-fuchsia-500/15 border border-fuchsia-500/30 text-fuchsia-300"
                              }`}
                            >
                              {isFlowseal ? t.strategies.engineFlowseal : t.strategies.engineZapret2}
                            </span>
                          </div>
                          <div className="text-[11px] text-ink4 mt-1 line-clamp-2 leading-relaxed min-h-[32px]">
                            {friendlyDesc}
                          </div>
                          <div className="text-[10px] font-mono text-ink5 mt-1 truncate">
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
                          <div className="flex h-4 w-4 items-center justify-center rounded-full bg-violet-500 text-ink">
                            <Check className="h-2.5 w-2.5" />
                          </div>
                        )}
                      </div>
                    </div>
                  </div>

                  {/* Bottom Action Footer */}
                  <div className="mt-3 pt-3 border-t t-divider flex items-center justify-between gap-2">
                    {/* Left: Custom Actions or Strategy ID Badge */}
                    <div className="flex items-center gap-1 min-w-0">
                      {isCustom ? (
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            onClick={(e) => handleEditCustom(strat, e)}
                            disabled={isTransitioning}
                            className="p-1.5 rounded-control text-ink4 hover:text-violet-300 hover:bg-raise transition-colors cursor-pointer"
                            title={t.strategies.editCustom}
                          >
                            <Pencil className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={(e) => handleDeleteCustom(strat.id, e)}
                            disabled={isTransitioning}
                            className="p-1.5 rounded-control text-ink4 hover:text-rose-400 hover:bg-rose-500/10 transition-colors cursor-pointer"
                            title={t.strategies.deleteCustom}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      ) : (
                        <span className="text-[10px] text-ink4 font-mono truncate max-w-[105px] bg-raise border t-divider px-1.5 py-0.5 rounded-chip">
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
                          className="flex items-center gap-1.5 rounded-panel bg-rose-500/15 border border-rose-500/35 hover:bg-rose-500/25 text-rose-300 px-3 py-1.5 text-xs font-bold transition-all active:scale-95 cursor-pointer disabled:opacity-50"
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
                          className="flex items-center gap-1.5 rounded-panel bg-gradient-to-r from-accent-fill to-accent hover:from-accent-fill-hover hover:to-accent text-ink px-3 py-1.5 text-xs font-bold shadow-md shadow-accent-fill/20 transition-all active:scale-95 cursor-pointer disabled:opacity-50"
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
                          className={`flex items-center gap-1.5 rounded-panel px-3 py-1.5 text-xs font-bold transition-all active:scale-95 cursor-pointer disabled:opacity-50 ${
                            isSelected
                              ? "bg-accent-fill hover:bg-accent text-ink shadow-md shadow-violet-600/25"
                              : "bg-raise hover:bg-hovert text-ink2 border border-line"
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
      </div>
    );
  }
);
