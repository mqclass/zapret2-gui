import React from "react";
import { Gamepad2, Shield, Filter, Check, Sliders } from "lucide-react";
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
    badge: string;
    icon: React.ReactNode;
    color: string;
    activeBorder: string;
    activeBg: string;
  }[] = [
    {
      id: "full",
      label: t.splitTunnel.modeFull,
      desc: t.splitTunnel.modeFullDesc,
      badge: t.splitTunnel.badgeFull,
      icon: <Shield className="h-4 w-4" />,
      color: "text-blue-400",
      activeBorder: "border-blue-500/50",
      activeBg: "bg-blue-600/10 text-blue-200",
    },
    {
      id: "strict",
      label: t.splitTunnel.modeStrict,
      desc: t.splitTunnel.modeStrictDesc,
      badge: t.splitTunnel.badgeStrict,
      icon: <Filter className="h-4 w-4" />,
      color: "text-emerald-400",
      activeBorder: "border-emerald-500/50",
      activeBg: "bg-emerald-600/10 text-emerald-200",
    },
    {
      id: "game_mode",
      label: t.splitTunnel.modeGame,
      desc: t.splitTunnel.modeGameDesc,
      badge: t.splitTunnel.badgeGame,
      icon: <Gamepad2 className="h-4 w-4" />,
      color: "text-purple-400",
      activeBorder: "border-purple-500/50",
      activeBg: "bg-purple-600/10 text-purple-200",
    },
  ];

  return (
    <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-3.5 backdrop-blur-md">
      <div className="flex flex-wrap items-center justify-between gap-2 mb-2.5">
        <div className="flex items-center gap-2">
          <Sliders className="h-4 w-4 text-cyan-400" />
          <span className="text-xs font-semibold text-slate-300">
            {t.splitTunnel.title}
          </span>
        </div>
        {isRunning && (
          <span className="text-[11px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 px-2 py-0.5 rounded-full font-medium">
            Перезапустите обход после смены режима
          </span>
        )}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5">
        {modes.map((m) => {
          const isSelected = currentMode === m.id;
          return (
            <button
              key={m.id}
              onClick={() => onModeChange(m.id)}
              disabled={disabled}
              className={`flex flex-col text-left p-3 rounded-xl border transition-all cursor-pointer relative ${
                isSelected
                  ? `${m.activeBorder} ${m.activeBg} shadow-md`
                  : "border-slate-800 bg-slate-950/60 hover:bg-slate-900 hover:border-slate-700/80 text-slate-400"
              } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center justify-between gap-1.5 mb-1">
                <div className="flex items-center gap-1.5 font-semibold text-xs">
                  <span className={isSelected ? m.color : "text-slate-500"}>
                    {m.icon}
                  </span>
                  <span className={isSelected ? "text-white" : "text-slate-300"}>
                    {m.badge}
                  </span>
                </div>
                {isSelected && (
                  <span className="h-4 w-4 rounded-full bg-emerald-500/20 text-emerald-400 flex items-center justify-center">
                    <Check className="h-3 w-3 stroke-[3]" />
                  </span>
                )}
              </div>
              <p className="text-[11px] leading-relaxed text-slate-400 line-clamp-2">
                {m.desc}
              </p>
            </button>
          );
        })}
      </div>
    </div>
  );
};
