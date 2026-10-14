import React from "react";
import { Gamepad2, Shield, Filter, Check, SlidersHorizontal } from "lucide-react";
import { SplitTunnelMode } from "../types";
import { useI18n } from "../i18n";

interface SplitTunnelBarProps {
  currentMode: SplitTunnelMode;
  onModeChange: (mode: SplitTunnelMode) => void;
  isRunning: boolean;
  disabled?: boolean;
}

export const SplitTunnelBar: React.FC<SplitTunnelBarProps> = ({
  currentMode,
  onModeChange,
  isRunning,
  disabled = false,
}) => {
  const { t } = useI18n();

  const modes: {
    id: SplitTunnelMode;
    label: string;
    desc: string;
    icon: React.ReactNode;
  }[] = [
    {
      id: "full",
      label: t.splitTunnel.badgeFull,
      desc: "Все сайты и сервисы через WinWS",
      icon: <Shield className="h-3.5 w-3.5" />,
    },
    {
      id: "strict",
      label: t.splitTunnel.badgeStrict,
      desc: "Только списки блокировок",
      icon: <Filter className="h-3.5 w-3.5" />,
    },
    {
      id: "game_mode",
      label: t.splitTunnel.badgeGame,
      desc: "Мин. пинг: игры в обход фильтра",
      icon: <Gamepad2 className="h-3.5 w-3.5" />,
    },
  ];

  return (
    <div className="rounded-xl border border-[#202534] bg-[#12151d] p-3 space-y-2.5">
      <div className="flex items-center justify-between text-xs">
        <div className="flex items-center gap-2">
          <SlidersHorizontal className="h-3.5 w-3.5 text-blue-400" />
          <span className="font-semibold text-slate-300">
            {t.splitTunnel.title}
          </span>
        </div>
        {isRunning && (
          <span className="text-[10px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 px-2 py-0.5 rounded-md font-medium">
            Перезапустите обход для применения
          </span>
        )}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-2">
        {modes.map((m) => {
          const isSelected = currentMode === m.id;
          return (
            <button
              key={m.id}
              type="button"
              onClick={() => onModeChange(m.id)}
              disabled={disabled}
              className={`flex flex-col text-left p-2.5 rounded-lg border transition-all cursor-pointer ${
                isSelected
                  ? "bg-[#1d2332] border-[#384359] text-white shadow-sm"
                  : "bg-[#151922] border-[#222735] hover:bg-[#1a1f2b] hover:border-[#2d3445] text-slate-400"
              } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <div className="flex items-center gap-1.5">
                  <span className={isSelected ? "text-blue-400" : "text-slate-500"}>
                    {m.icon}
                  </span>
                  <span className={`text-xs font-semibold ${isSelected ? "text-white" : "text-slate-300"}`}>
                    {m.label}
                  </span>
                </div>
                {isSelected && (
                  <span className="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-blue-500/20 text-blue-400">
                    <Check className="h-2.5 w-2.5 stroke-[3]" />
                  </span>
                )}
              </div>
              <p className="text-[11px] text-slate-400 mt-1 leading-normal font-normal">
                {m.desc}
              </p>
            </button>
          );
        })}
      </div>
    </div>
  );
};
