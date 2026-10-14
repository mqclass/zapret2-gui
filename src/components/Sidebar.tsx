import React from "react";
import {
  Shield,
  Zap,
  Activity,
  Globe2,
  ListFilter,
  Terminal,
  Settings,
  Flame,
  CheckCircle2,
  XCircle,
  Loader2,
  Gamepad2,
  Layers,
  Compass,
} from "lucide-react";
import { useI18n } from "../i18n";
import { SupervisorState, SplitTunnelMode } from "../types";

export type NavTab =
  | "dashboard"
  | "strategies"
  | "diagnostics"
  | "dns"
  | "lists"
  | "logs"
  | "settings";

interface SidebarProps {
  activeTab: NavTab;
  onTabChange: (tab: NavTab) => void;
  state: SupervisorState;
  splitTunnelMode: SplitTunnelMode;
  isElevated: boolean;
}

export const Sidebar: React.FC<SidebarProps> = React.memo(
  ({ activeTab, onTabChange, state, splitTunnelMode, isElevated }) => {
    const { t } = useI18n();

    const isRunning = state === "running";
    const isStarting = state === "starting";
    const isStopping = state === "stopping";

    const navItems: { id: NavTab; label: string; icon: React.ReactNode; badge?: string }[] = [
      {
        id: "dashboard",
        label: t.nav.dashboard,
        icon: <Shield className="h-4 w-4" />,
      },
      {
        id: "strategies",
        label: t.nav.strategies,
        icon: <Zap className="h-4 w-4" />,
      },
      {
        id: "diagnostics",
        label: t.nav.diagnostics,
        icon: <Activity className="h-4 w-4" />,
      },
      {
        id: "dns",
        label: t.nav.dns,
        icon: <Globe2 className="h-4 w-4" />,
      },
      {
        id: "lists",
        label: t.nav.lists,
        icon: <ListFilter className="h-4 w-4" />,
      },
      {
        id: "logs",
        label: t.nav.logs,
        icon: <Terminal className="h-4 w-4" />,
      },
      {
        id: "settings",
        label: t.nav.settings,
        icon: <Settings className="h-4 w-4" />,
      },
    ];

    return (
      <aside className="w-56 shrink-0 flex flex-col justify-between border-r border-white/[0.07] bg-slate-950/90 backdrop-blur-2xl select-none">
        {/* Top Brand Header */}
        <div className="p-4 border-b border-white/[0.05]">
          <div className="flex items-center gap-3">
            <div className="relative flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-blue-600 via-indigo-600 to-cyan-500 shadow-md shadow-blue-500/25">
              <Flame className="h-5 w-5 text-white" />
              {/* Animated status ring */}
              <span
                className={`absolute -bottom-0.5 -right-0.5 flex h-3.5 w-3.5 items-center justify-center rounded-full border-2 border-slate-950 ${
                  isRunning
                    ? "bg-emerald-500"
                    : isStarting || isStopping
                    ? "bg-amber-500"
                    : "bg-slate-600"
                }`}
              >
                {isRunning && (
                  <span className="absolute h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
                )}
              </span>
            </div>
            <div>
              <div className="flex items-center gap-1.5">
                <span className="text-base font-bold tracking-tight text-white">zapret2</span>
                <span className="rounded-md bg-blue-500/15 border border-blue-500/30 px-1.5 py-0.2 text-[10px] font-mono font-bold text-blue-400">
                  v1.0.8
                </span>
              </div>
              <p className="text-[11px] font-medium text-slate-400 truncate">GUI & Supervisor</p>
            </div>
          </div>
        </div>

        {/* Navigation List */}
        <nav className="flex-1 p-3 space-y-1 overflow-y-auto">
          {navItems.map((item) => {
            const isActive = activeTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onTabChange(item.id)}
                className={`w-full flex items-center justify-between px-3 py-2.5 rounded-xl text-xs font-semibold transition-all cursor-pointer ${
                  isActive
                    ? "bg-gradient-to-r from-blue-600 to-indigo-600 text-white shadow-md shadow-blue-600/20"
                    : "text-slate-400 hover:text-slate-200 hover:bg-white/[0.04]"
                }`}
              >
                <div className="flex items-center gap-2.5">
                  <span className={isActive ? "text-white" : "text-slate-400"}>{item.icon}</span>
                  <span>{item.label}</span>
                </div>
                {item.id === "dashboard" && (
                  <span
                    className={`h-2 w-2 rounded-full ${
                      isRunning
                        ? "bg-emerald-400 shadow-[0_0_8px_#34d399]"
                        : isStarting || isStopping
                        ? "bg-amber-400 animate-pulse"
                        : "bg-slate-600"
                    }`}
                  />
                )}
              </button>
            );
          })}
        </nav>

        {/* Bottom Status Card */}
        <div className="p-3 border-t border-white/[0.05] space-y-2">
          {/* Active Mode Pill */}
          <div className="flex items-center justify-between rounded-lg bg-white/[0.03] border border-white/[0.05] px-2.5 py-1.5 text-[11px]">
            <span className="text-slate-400 font-medium">Режим:</span>
            <div className="flex items-center gap-1 font-semibold text-slate-200">
              {splitTunnelMode === "game_mode" ? (
                <>
                  <Gamepad2 className="h-3 w-3 text-amber-400" />
                  <span className="text-amber-300">Game Mode</span>
                </>
              ) : splitTunnelMode === "strict" ? (
                <>
                  <Compass className="h-3 w-3 text-cyan-400" />
                  <span className="text-cyan-300">Браузеры</span>
                </>
              ) : (
                <>
                  <Layers className="h-3 w-3 text-indigo-400" />
                  <span className="text-indigo-300">Полный</span>
                </>
              )}
            </div>
          </div>

          {/* Engine Status Line */}
          <div className="flex items-center justify-between rounded-lg bg-white/[0.03] border border-white/[0.05] px-2.5 py-1.5 text-[11px]">
            <span className="text-slate-400 font-medium">Служба:</span>
            <span
              className={`font-semibold flex items-center gap-1 ${
                isRunning
                  ? "text-emerald-400"
                  : isStarting || isStopping
                  ? "text-amber-400"
                  : "text-slate-500"
              }`}
            >
              {isStarting || isStopping ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : isRunning ? (
                <CheckCircle2 className="h-3 w-3" />
              ) : (
                <XCircle className="h-3 w-3" />
              )}
              {isRunning
                ? "Активна"
                : isStarting
                ? "Запуск..."
                : isStopping
                ? "Остановка..."
                : "Отключена"}
            </span>
          </div>
        </div>
      </aside>
    );
  }
);
