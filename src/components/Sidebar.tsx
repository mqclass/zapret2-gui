import React, { useCallback } from "react";
import { motion } from "framer-motion";
import {
  Shield,
  Zap,
  Activity,
  Globe2,
  ListFilter,
  Terminal,
  Send,
  Settings,
  Gamepad2,
  Layers,
  Compass,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { ZapretLogo } from "./ZapretLogo";
import { useI18n } from "../i18n";
import { SupervisorState, SplitTunnelMode } from "../types";

export type NavTab =
  | "dashboard"
  | "strategies"
  | "diagnostics"
  | "dns"
  | "lists"
  | "logs"
  | "telegram"
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

    const navItems: { id: NavTab; label: string; icon: React.ReactNode }[] = [
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
        id: "telegram",
        label: t.nav.telegram,
        icon: <Send className="h-4 w-4" />,
      },
      {
        id: "settings",
        label: t.nav.settings,
        icon: <Settings className="h-4 w-4" />,
      },
    ];

    const handleHeaderMouseDown = useCallback((e: React.MouseEvent) => {
      if (
        e.button === 0 &&
        !(e.target as HTMLElement).closest("button, input, select, a")
      ) {
        getCurrentWindow()
          .startDragging()
          .catch(() => {
            invoke("window_start_dragging").catch(() => {});
          });
      }
    }, []);

    return (
      <aside className="glass-sidebar w-60 shrink-0 flex flex-col justify-between select-none">
        {/* Top Brand Header (Draggable) */}
        <div
          data-tauri-drag-region
          onMouseDown={handleHeaderMouseDown}
          className="p-4 border-b t-divider cursor-default"
        >
          <div data-tauri-drag-region className="flex items-center gap-3">
            <div className="relative flex h-9 w-9 items-center justify-center rounded-panel bg-raise border border-line">
              <ZapretLogo className="h-5 w-5" glow={isRunning} />
              {/* Status dot */}
              <span
                className={`absolute -bottom-0.5 -right-0.5 flex h-2.5 w-2.5 items-center justify-center rounded-full border-2 border-sidebar ${
                  isRunning
                    ? "bg-emerald-500"
                    : isStarting || isStopping
                    ? "bg-amber-500"
                    : "bg-ink5"
                }`}
              />
            </div>
            <div data-tauri-drag-region>
              <div data-tauri-drag-region className="flex items-center gap-1.5">
                <span data-tauri-drag-region className="text-sm font-bold tracking-tight text-ink">zapret2</span>
                <span className="rounded-chip bg-accent-wash border border-accent-line px-1 py-0.5 text-[9px] font-mono font-semibold text-accent">
                  {t.header.version}
                </span>
              </div>
              <p data-tauri-drag-region className="text-[10px] font-medium text-ink4">{t.sidebar.tagline}</p>
            </div>
          </div>
        </div>

        {/* Navigation List */}
        <nav className="flex-1 p-2.5 space-y-1 overflow-y-auto">
          {navItems.map((item) => {
            const isActive = activeTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onTabChange(item.id)}
                className={`t-navitem ${isActive ? "t-navitem-active" : ""}`}
              >
                {isActive && (
                  <motion.span
                    layoutId="sidebar-active-pill"
                    className="nav-active-pill"
                    transition={{ type: "spring", stiffness: 380, damping: 32, mass: 0.9 }}
                  />
                )}
                <span className="relative flex items-center gap-2.5">
                  <span className="t-navitem-icon">
                    {item.icon}
                  </span>
                  <span>{item.label}</span>
                </span>
                {item.id === "dashboard" && (
                  <span
                    className={`relative h-1.5 w-1.5 rounded-full ${
                      isRunning
                        ? "bg-emerald-400"
                        : isStarting || isStopping
                        ? "bg-amber-400 animate-pulse"
                        : "bg-ink5"
                    }`}
                  />
                )}
              </button>
            );
          })}
        </nav>

        {/* Bottom Status Card */}
        <div className="p-3 border-t t-divider space-y-1.5">
          {/* Active Mode Pill */}
          <div className="t-card-in flex items-center justify-between px-2.5 py-1.5 text-[11px]">
            <span className="text-ink4 font-medium">{t.sidebar.modeLabel}</span>
            <div className="flex items-center gap-1 font-semibold text-ink2">
              {splitTunnelMode === "game_mode" ? (
                <>
                  <Gamepad2 className="h-3 w-3 text-purple-400" />
                  <span>{t.sidebar.modeGame}</span>
                </>
              ) : splitTunnelMode === "strict" ? (
                <>
                  <Compass className="h-3 w-3 text-emerald-400" />
                  <span>{t.sidebar.modeStrict}</span>
                </>
              ) : (
                <>
                  <Layers className="h-3 w-3 text-accent" />
                  <span>{t.sidebar.modeFull}</span>
                </>
              )}
            </div>
          </div>

          {/* Engine Status Pill */}
          <div className="t-card-in flex items-center justify-between px-2.5 py-1.5 text-[11px]">
            <span className="text-ink4 font-medium">{t.sidebar.serviceLabel}</span>
            <div className="flex items-center gap-1.5">
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  isRunning
                    ? "bg-emerald-400"
                    : isStarting || isStopping
                    ? "bg-amber-400 animate-pulse"
                    : "bg-ink5"
                }`}
              />
              <span
                className={`font-semibold ${
                  isRunning
                    ? "text-emerald-400"
                    : isStarting || isStopping
                    ? "text-amber-400"
                    : "text-ink4"
                }`}
              >
                {isRunning
                  ? t.sidebar.active
                  : isStarting
                  ? t.sidebar.starting
                  : isStopping
                  ? t.sidebar.stopping
                  : t.sidebar.inactive}
              </span>
            </div>
          </div>
        </div>
      </aside>
    );
  }
);
