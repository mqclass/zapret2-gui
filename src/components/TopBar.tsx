import React, { useState, useRef, useEffect, useCallback } from "react";
import {
  ShieldCheck,
  ShieldAlert,
  Rocket,
  Trash2,
  Globe,
  RefreshCw,
  ChevronDown,
  Check,
  Minus,
  Square,
  X,
} from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";
import { Language } from "../i18n/types";
import { NavTab } from "./Sidebar";

interface TopBarProps {
  activeTab: NavTab;
  isElevated: boolean;
  onRelaunchElevated: () => void;
  isAutostart: boolean;
  onToggleAutostart: () => void;
  isTogglingAutostart: boolean;
  onKillStray: () => void;
  isStrayCleaning: boolean;
}

export const TopBar: React.FC<TopBarProps> = React.memo(
  ({
    activeTab,
    isElevated,
    onRelaunchElevated,
    isAutostart,
    onToggleAutostart,
    isTogglingAutostart,
    onKillStray,
    isStrayCleaning,
  }) => {
    const { language, setLanguage, t } = useI18n();
    const [isLangMenuOpen, setIsLangMenuOpen] = useState(false);
    const langDropdownRef = useRef<HTMLDivElement>(null);

    // Close language dropdown on outside click
    useEffect(() => {
      const handleOutsideClick = (e: MouseEvent) => {
        if (
          langDropdownRef.current &&
          !langDropdownRef.current.contains(e.target as Node)
        ) {
          setIsLangMenuOpen(false);
        }
      };
      if (isLangMenuOpen) {
        document.addEventListener("mousedown", handleOutsideClick);
      }
      return () => {
        document.removeEventListener("mousedown", handleOutsideClick);
      };
    }, [isLangMenuOpen]);

    const getTabMeta = (tab: NavTab) => {
      switch (tab) {
        case "dashboard":
          return {
            title: t.tabMeta.dashboardTitle,
            subtitle: t.tabMeta.dashboardSubtitle,
          };
        case "strategies":
          return {
            title: t.tabMeta.strategiesTitle,
            subtitle: t.tabMeta.strategiesSubtitle,
          };
        case "diagnostics":
          return {
            title: t.tabMeta.diagnosticsTitle,
            subtitle: t.tabMeta.diagnosticsSubtitle,
          };
        case "dns":
          return {
            title: t.tabMeta.dnsTitle,
            subtitle: t.tabMeta.dnsSubtitle,
          };
        case "lists":
          return {
            title: t.tabMeta.listsTitle,
            subtitle: t.tabMeta.listsSubtitle,
          };
        case "logs":
          return {
            title: t.tabMeta.logsTitle,
            subtitle: t.tabMeta.logsSubtitle,
          };
        case "settings":
          return {
            title: t.tabMeta.settingsTitle,
            subtitle: t.tabMeta.settingsSubtitle,
          };
      }
    };

    const meta = getTabMeta(activeTab);

    const languages: { code: Language; label: string; full: string }[] = [
      { code: "ru", label: "RU", full: t.header.langRu },
      { code: "en", label: "EN", full: t.header.langEn },
      { code: "uk", label: "UK", full: t.header.langUk },
    ];

    const currentLangItem = languages.find((l) => l.code === language) || languages[0];

    // Native window controls
    const handleMinimize = useCallback(() => {
      getCurrentWindow().minimize().catch(() => {
        invoke("window_minimize").catch((e) => console.error("Minimize error:", e));
      });
    }, []);

    const handleToggleMaximize = useCallback(() => {
      getCurrentWindow().toggleMaximize().catch(() => {
        invoke("window_toggle_maximize").catch((e) => console.error("Maximize error:", e));
      });
    }, []);

    const handleClose = useCallback(() => {
      getCurrentWindow().hide().catch(() => {
        invoke("window_close").catch((e) => console.error("Close error:", e));
      });
    }, []);

    // Native window drag handler for custom frameless window
    const handleDragMouseDown = useCallback((e: React.MouseEvent) => {
      if (
        e.button === 0 &&
        !(e.target as HTMLElement).closest("button, input, select, a, [role='button']")
      ) {
        getCurrentWindow()
          .startDragging()
          .catch(() => {
            invoke("window_start_dragging").catch((err) =>
              console.error("window_start_dragging error:", err)
            );
          });
      }
    }, []);

    return (
      <header
        data-tauri-drag-region
        onMouseDown={handleDragMouseDown}
        className="glass-bar relative z-50 h-14 shrink-0 flex items-center justify-between pl-5 pr-2 border-b border-line select-none"
      >
        {/* Title and subtitle area (Draggable + double-click to maximize) */}
        <div
          data-tauri-drag-region
          onDoubleClick={handleToggleMaximize}
          className="flex items-center gap-3"
        >
          <div data-tauri-drag-region>
            <h2 data-tauri-drag-region className="text-sm font-semibold tracking-tight text-ink leading-tight">
              {meta.title}
            </h2>
            <p data-tauri-drag-region className="text-[11px] text-ink4 font-normal">
              {meta.subtitle}
            </p>
          </div>
        </div>

        {/* Right side actions and window controls */}
        <div className="flex items-center gap-2">
          {/* UAC Elevation Status */}
          {isElevated ? (
            <div
              className="flex items-center gap-1.5 rounded-control border border-emerald-500/25 bg-emerald-500/10 px-2.5 py-1 text-[11px] font-medium text-emerald-300"
              title={t.header.adminTooltip}
            >
              <ShieldCheck className="h-3.5 w-3.5 text-emerald-400" />
              <span>{t.header.adminShort}</span>
            </div>
          ) : (
            <button
              onClick={onRelaunchElevated}
              className="flex items-center gap-1.5 rounded-control border border-amber-500/35 bg-amber-500/10 px-2.5 py-1 text-[11px] font-medium text-amber-300 hover:bg-amber-500/20 transition-all cursor-pointer"
              title={t.header.adminRequiredTooltip}
            >
              <ShieldAlert className="h-3.5 w-3.5 text-amber-400" />
              <span>{t.header.adminRequiredShort}</span>
            </button>
          )}

          {/* Quick Autostart Toggle */}
          <button
            onClick={onToggleAutostart}
            disabled={isTogglingAutostart}
            className={`flex items-center gap-1.5 rounded-control border px-2.5 py-1 text-[11px] font-medium transition-all cursor-pointer disabled:opacity-60 ${
              isAutostart
                ? "bg-accent-wash border-accent-line text-accent-soft hover:bg-accent-wash"
                : "bg-white/[0.03] border-white/[0.08] text-ink4 hover:text-ink2 hover:bg-white/[0.06]"
            }`}
            title={isAutostart ? t.header.autostartOnTitle : t.header.autostartOffTitle}
          >
            {isTogglingAutostart ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
            ) : (
              <Rocket
                className={`h-3.5 w-3.5 ${isAutostart ? "text-accent" : "text-ink5"}`}
              />
            )}
            <span>{t.header.autostart}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isAutostart ? "bg-accent" : "bg-ink5"
              }`}
            />
          </button>

          {/* Quick Stray Process Cleaner */}
          <button
            onClick={onKillStray}
            disabled={isStrayCleaning}
            className="flex items-center gap-1.5 rounded-control border border-white/[0.08] bg-white/[0.03] px-2.5 py-1 text-[11px] font-medium text-ink4 hover:text-rose-300 hover:border-rose-500/30 hover:bg-rose-500/10 transition-all cursor-pointer disabled:opacity-60"
            title={t.header.cleanProcessesTooltip}
          >
            {isStrayCleaning ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-rose-400" />
            ) : (
              <Trash2 className="h-3.5 w-3.5" />
            )}
            <span className="hidden sm:inline">{t.header.cleanShort}</span>
          </button>

          {/* Animated Single Active Language Dropdown */}
          <div className="relative z-[60]" ref={langDropdownRef}>
            <button
              type="button"
              onClick={() => setIsLangMenuOpen((prev) => !prev)}
              className="flex items-center gap-1.5 rounded-control border border-line bg-raise hover:bg-hovert px-2.5 py-1 text-[11px] font-semibold text-ink2 transition-all cursor-pointer hover:border-linestrong"
              title={t.header.changeLanguage}
            >
              <Globe className="h-3.5 w-3.5 text-accent" />
              <span className="font-mono tracking-wider">{currentLangItem.label}</span>
              <ChevronDown
                className={`h-3 w-3 text-ink4 transition-transform duration-200 ${
                  isLangMenuOpen ? "rotate-180 text-accent" : ""
                }`}
              />
            </button>

            {/* Floating Dropdown Menu (Guaranteed on top of hero card) */}
            <div
              className={`absolute right-0 top-full mt-2 w-36 rounded-panel border border-line bg-raise shadow-2xl p-1 z-[100] transition-all duration-150 ease-out origin-top-right transform ${
                isLangMenuOpen
                  ? "opacity-100 scale-100 translate-y-0 pointer-events-auto"
                  : "opacity-0 scale-95 -translate-y-2 pointer-events-none"
              }`}
            >
              {languages.map((l) => {
                const isSelected = language === l.code;
                return (
                  <button
                    key={l.code}
                    type="button"
                    onClick={() => {
                      setLanguage(l.code);
                      setIsLangMenuOpen(false);
                    }}
                    className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-control text-xs font-medium transition-colors cursor-pointer ${
                      isSelected
                        ? "bg-accent-wash text-accent-soft font-semibold"
                        : "text-ink3 hover:text-ink hover:bg-white/[0.06]"
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[10px] text-ink4">
                        {l.label}
                      </span>
                      <span>{l.full}</span>
                    </div>
                    {isSelected && <Check className="h-3.5 w-3.5 text-accent" />}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Window Control Buttons (Frameless Unified Window) */}
          <div className="flex items-center ml-2 border-l border-white/[0.08] pl-2">
            <button
              onClick={handleMinimize}
              className="h-8 w-8 flex items-center justify-center rounded-control text-ink4 hover:text-ink hover:bg-white/[0.08] transition-colors cursor-pointer"
              title={t.header.minimizeTitle}
            >
              <Minus className="h-3.5 w-3.5" />
            </button>
            <button
              onClick={handleToggleMaximize}
              className="h-8 w-8 flex items-center justify-center rounded-control text-ink4 hover:text-ink hover:bg-white/[0.08] transition-colors cursor-pointer"
              title={t.header.maximizeTitle}
            >
              <Square className="h-3 w-3" />
            </button>
            <button
              onClick={handleClose}
              className="h-8 w-8 flex items-center justify-center rounded-control text-ink4 hover:text-ink hover:bg-rose-600 transition-colors cursor-pointer"
              title={t.header.closeTitle}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </header>
    );
  }
);
