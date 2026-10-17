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
      desc: t.splitTunnel.modeFullDesc,
      icon: <Shield className="h-3.5 w-3.5" />,
    },
    {
      id: "strict",
      label: t.splitTunnel.badgeStrict,
      desc: t.splitTunnel.modeStrictDesc,
      icon: <Filter className="h-3.5 w-3.5" />,
    },
    {
      id: "game_mode",
      label: t.splitTunnel.badgeGame,
      desc: t.splitTunnel.modeGameDesc,
      icon: <Gamepad2 className="h-3.5 w-3.5" />,
    },
  ];

  return (
    <div className="rounded-panel border border-line bg-cardin p-3 space-y-2.5">
      <div className="flex items-center justify-between text-xs">
        <div className="flex items-center gap-2">
          <SlidersHorizontal className="h-3.5 w-3.5 text-accent" />
          <span className="font-semibold text-ink3">
            {t.splitTunnel.title}
          </span>
        </div>
        {isRunning && (
          <span className="text-[10px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 px-2 py-0.5 rounded-chip font-medium">
            {t.splitTunnel.restartHint}
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
              className={`flex flex-col text-left p-2.5 rounded-control border transition-all cursor-pointer ${
                isSelected
                  ? "bg-hovert border-linestrong text-ink shadow-sm"
                  : "bg-cardin border-line hover:bg-hovert hover:border-linestrong text-ink4"
              } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
            >
              <div className="flex items-center justify-between gap-1.5">
                <div className="flex items-center gap-1.5">
                  <span className={isSelected ? "text-accent" : "text-ink5"}>
                    {m.icon}
                  </span>
                  <span className={`text-xs font-semibold ${isSelected ? "text-ink" : "text-ink3"}`}>
                    {m.label}
                  </span>
                </div>
                {isSelected && (
                  <span className="flex h-3.5 w-3.5 items-center justify-center rounded-full bg-accent-wash text-accent">
                    <Check className="h-2.5 w-2.5 stroke-[3]" />
                  </span>
                )}
              </div>
              <p className="text-[11px] text-ink4 mt-1 leading-normal font-normal">
                {m.desc}
              </p>
            </button>
          );
        })}
      </div>
    </div>
  );
};
