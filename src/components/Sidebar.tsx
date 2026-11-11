import React, { useCallback } from "react";
import {
  Shield,
  Zap,
  Activity,
  Globe2,
  Settings,
  Gamepad2,
  Layers,
  Compass,
  Eye,
  Code2,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import logoUrl from "../assets/logo.png";
import { useI18n } from "../i18n";
import { SupervisorState, SplitTunnelMode } from "../types";

export type NavTab =
  | "dashboard"
  | "presets"
  | "dns"
  | "diagnostics"
  | "settings";

interface SidebarProps {
  activeTab: NavTab;
  onTabChange: (tab: NavTab) => void;
  state: SupervisorState;
  splitTunnelMode: SplitTunnelMode;
  isElevated: boolean;
  expertMode: boolean;
  onExpertModeChange: (expert: boolean) => void;
}

export const Sidebar: React.FC<SidebarProps> = React.memo(
  ({
    activeTab,
    onTabChange,
    state,
    splitTunnelMode,
    expertMode,
    onExpertModeChange,
  }) => {
    const { t } = useI18n();

    const isRunning = state === "running";
    const isStarting = state === "starting";
    const isStopping = state === "stopping";

    const navItems: {
      id: NavTab;
      label: string;
      sub: string;
      icon: React.ReactNode;
    }[] = [
      {
        id: "dashboard",
        label: t.nav.dashboard,
        sub: t.nav.dashboardSub,
        icon: <Shield className="h-4 w-4" />,
      },
      {
        id: "presets",
        label: t.nav.presets,
        sub: t.nav.presetsSub,
        icon: <Zap className="h-4 w-4" />,
      },
      {
        id: "dns",
        label: t.nav.dns,
        sub: t.nav.dnsSub,
        icon: <Globe2 className="h-4 w-4" />,
      },
      {
        id: "diagnostics",
        label: t.nav.diagnostics,
        sub: t.nav.diagnosticsSub,
        icon: <Activity className="h-4 w-4" />,
      },
      {
        id: "settings",
        label: t.nav.settings,
        sub: t.nav.settingsSub,
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
            <div className="relative h-9 w-9 shrink-0">
              {/* Project logo (the bird from the app icon), full-bleed
                  like the icon itself. The clip lives on this inner box
                  only: the status dot below overhangs the corner by 2px
                  and must not be cut by it. */}
              <div className="flex h-full w-full items-center justify-center overflow-hidden rounded-panel border border-line">
                <img
                  src={logoUrl}
                  alt="zapret2"
                  draggable={false}
                  data-tauri-drag-region
                  className="h-full w-full object-cover"
                />
              </div>
              {/* Status dot */}
              <span
                className={`absolute -bottom-0.5 -right-0.5 flex h-2.5 w-2.5 items-center justify-center rounded-full border-2 border-sidebar ${
                  isRunning
                    ? "bg-emerald-500 shadow-[0_0_8px_rgba(52,211,153,0.9)]"
                    : isStarting || isStopping
                    ? "bg-amber-500 animate-pulse"
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
            </div>
          </div>
        </div>

        {/* View Mode Switch (Простой вид / Эксперт) */}
        <div className="px-3 pt-3">
          <div className="t-modeswitch" role="group" aria-label={t.nav.simpleView}>
            <button
              type="button"
              onClick={() => onExpertModeChange(false)}
              className={`t-modeswitch-btn ${!expertMode ? "t-modeswitch-btn-active" : ""}`}
            >
              <Eye className="h-3.5 w-3.5" />
              <span>{t.nav.simpleView}</span>
            </button>
            <button
              type="button"
              onClick={() => onExpertModeChange(true)}
              className={`t-modeswitch-btn ${expertMode ? "t-modeswitch-btn-active" : ""}`}
            >
              <Code2 className="h-3.5 w-3.5" />
              <span>{t.nav.expertView}</span>
            </button>
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
                className={`t-navitem t-navitem-lg ${isActive ? "t-navitem-active" : ""}`}
              >
                <span className="relative flex items-center gap-2.5 min-w-0">
                  <span className="t-navitem-tile">{item.icon}</span>
                  <span className="flex flex-col items-start min-w-0">
                    <span className="text-[13px] font-semibold leading-tight whitespace-nowrap">
                      {item.label}
                    </span>
                    <span className="text-[10.5px] font-normal text-ink4 truncate mt-0.5">
                      {item.sub}
                    </span>
                  </span>
                </span>
                {item.id === "dashboard" && (
                  <span
                    className={`relative shrink-0 inline-flex rounded-full border px-1.5 py-0.5 text-[9.5px] font-bold ${
                      isRunning
                        ? "border-emerald-400/30 bg-emerald-500/20 text-emerald-300"
                        : "border-amber-400/30 bg-amber-500/20 text-amber-300"
                    } ${isStarting || isStopping ? "animate-pulse" : ""}`}
                  >
                    {isRunning ? "ON" : "OFF"}
                  </span>
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
