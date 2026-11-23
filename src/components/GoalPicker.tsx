import React from "react";
import { Check, Globe2, MessageCircle, Settings2, Youtube } from "lucide-react";
import { LuaStrategy } from "../types";
import { useI18n } from "../i18n";

interface GoalPickerProps {
  strategies: LuaStrategy[];
  selectedStrategyId: string;
  disabled?: boolean;
  onSelectStrategy: (id: string) => void;
  onStartStrategy: (id: string) => void;
  /** «Свой конфиг» goal: the host opens its custom-profile modal. */
  onCustomGoal: () => void;
  /** Dense one-line-per-goal layout for the simple dashboard hero flow;
      descriptions live on the presets tab. */
  compact?: boolean;
}

/**
 * Goal picker: plain-language entry point for ordinary users
 * («Что хотите разблокировать?»). Shared by the strategies tab and the
 * simple dashboard; the pick maps to a shipped preset and starts it,
 * the custom goal defers to the host's profile modal.
 */
export const GoalPicker: React.FC<GoalPickerProps> = React.memo(
  ({
    strategies,
    selectedStrategyId,
    disabled = false,
    onSelectStrategy,
    onStartStrategy,
    onCustomGoal,
    compact = false,
  }) => {
    const { t } = useI18n();

    return (
      <div className={`t-card ${compact ? "p-4" : "p-5"}`}>
        <div className="mb-3">
          <h2 className="text-sm font-semibold text-ink">{t.strategies.goalTitle}</h2>
          {/* Compact (dashboard hero, simple presets): the tiles are
              self-explanatory, the subtitle is vertical noise. */}
          {!compact && (
            <p className="text-xs text-ink4 mt-0.5">{t.strategies.goalSubtitle}</p>
          )}
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
                    onCustomGoal();
                  }
                }}
                className={`group flex flex-col ${
                  compact ? "gap-1.5 p-3" : "gap-2 p-3.5"
                } rounded-panel border text-left transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-60 ${
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
                  {!compact && (
                    <p className="text-[11px] text-ink4 mt-0.5 leading-snug">{g.desc}</p>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      </div>
    );
  }
);
