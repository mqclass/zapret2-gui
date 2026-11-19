import React, { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  Activity,
  Code2,
  Eye,
  Globe,
  LayoutDashboard,
  Play,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Square,
  Trash2,
  Zap,
} from "lucide-react";
import { LuaStrategy } from "../types";
import { NavTab } from "./Sidebar";
import { useI18n } from "../i18n";

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  isRunning: boolean;
  isTransitioning: boolean;
  isAutoTuning: boolean;
  isRefreshingProbes: boolean;
  isStrayCleaning: boolean;
  expertMode: boolean;
  strategies: LuaStrategy[];
  onStart: () => void;
  onStop: () => void;
  onAutoTune: () => void;
  onRefreshProbes: () => void;
  onKillStray: () => void;
  onToggleExpert: () => void;
  onNavigateTab: (tab: NavTab) => void;
  onStartStrategy: (strategyId: string) => void;
}

interface PaletteAction {
  id: string;
  group: "actions" | "navigate" | "strategies";
  title: string;
  hint: string;
  keywords: string;
  icon: React.ReactNode;
  disabled?: boolean;
  run: () => void;
}

/**
 * Command palette (Solaris port, Ctrl/Cmd+K): every real action in one
 * keyboard-driven list — start/stop, autopick, probe refresh, stray
 * cleanup, expert toggle, tab navigation, and starting any strategy
 * by name. No fake entries: each action calls the same handler the
 * corresponding button uses.
 *
 * Strategies only appear once the user types: with 38 profiles the
 * list is searchable, not browsable.
 */
export const CommandPalette: React.FC<CommandPaletteProps> = React.memo(
  ({
    open,
    onClose,
    isRunning,
    isTransitioning,
    isAutoTuning,
    isRefreshingProbes,
    isStrayCleaning,
    expertMode,
    strategies,
    onStart,
    onStop,
    onAutoTune,
    onRefreshProbes,
    onKillStray,
    onToggleExpert,
    onNavigateTab,
    onStartStrategy,
  }) => {
    const { t } = useI18n();
    const [query, setQuery] = useState("");
    const [selected, setSelected] = useState(0);
    const inputRef = useRef<HTMLInputElement>(null);
    const listRef = useRef<HTMLDivElement>(null);

    const actions = useMemo<PaletteAction[]>(() => {
      const list: PaletteAction[] = [
        isRunning
          ? {
              id: "stop",
              group: "actions",
              title: t.palette.stopBypass,
              hint: t.palette.stopHint,
              keywords: "стоп выключить остановить stop",
              icon: <Square className="h-4 w-4 text-rose-400" />,
              disabled: isTransitioning,
              run: onStop,
            }
          : {
              id: "start",
              group: "actions",
              title: t.palette.startBypass,
              hint: t.palette.startHint,
              keywords: "старт включить запустить start",
              icon: <Play className="h-4 w-4 text-emerald-400" />,
              disabled: isTransitioning,
              run: onStart,
            },
        {
          id: "autotune",
          group: "actions",
          title: t.palette.autoTune,
          hint: t.palette.autoTuneHint,
          keywords: "автоподбор подобрать автоматически автотюн auto",
          icon: <Sparkles className="h-4 w-4 text-accent" />,
          disabled: isAutoTuning || isTransitioning,
          run: onAutoTune,
        },
        {
          id: "probes",
          group: "actions",
          title: t.palette.checkServices,
          hint: t.palette.checkServicesHint,
          keywords: "проверить сервисы отклик проверка ping",
          icon: <RefreshCw className="h-4 w-4 text-accent" />,
          disabled: isRefreshingProbes,
          run: onRefreshProbes,
        },
        {
          id: "stray",
          group: "actions",
          title: t.palette.clearStuck,
          hint: t.palette.clearStuckHint,
          keywords: "очистить зависшие процессы winws stray",
          icon: <Trash2 className="h-4 w-4 text-rose-400" />,
          disabled: isStrayCleaning,
          run: onKillStray,
        },
        expertMode
          ? {
              id: "expert-off",
              group: "actions",
              title: t.palette.expertOff,
              hint: t.palette.expertOffHint,
              keywords: "эксперт простой вид expert",
              icon: <Eye className="h-4 w-4 text-accent" />,
              run: onToggleExpert,
            }
          : {
              id: "expert-on",
              group: "actions",
              title: t.palette.expertOn,
              hint: t.palette.expertOnHint,
              keywords: "эксперт режим expert",
              icon: <Code2 className="h-4 w-4 text-accent" />,
              run: onToggleExpert,
            },
        {
          id: "nav-dashboard",
          group: "navigate",
          title: t.nav.dashboard,
          hint: t.nav.dashboardSub,
          keywords: "главная dashboard",
          icon: <LayoutDashboard className="h-4 w-4 text-ink3" />,
          run: () => onNavigateTab("dashboard"),
        },
        {
          id: "nav-presets",
          group: "navigate",
          title: t.nav.presets,
          hint: t.nav.presetsSub,
          keywords: "пресеты хосты стратегии presets",
          icon: <Zap className="h-4 w-4 text-ink3" />,
          run: () => onNavigateTab("presets"),
        },
        {
          id: "nav-dns",
          group: "navigate",
          title: t.nav.dns,
          hint: t.nav.dnsSub,
          keywords: "ускорение dns телеграм dns",
          icon: <Globe className="h-4 w-4 text-ink3" />,
          run: () => onNavigateTab("dns"),
        },
        {
          id: "nav-diagnostics",
          group: "navigate",
          title: t.nav.diagnostics,
          hint: t.nav.diagnosticsSub,
          keywords: "диагностика логи diagnostics",
          icon: <Activity className="h-4 w-4 text-ink3" />,
          run: () => onNavigateTab("diagnostics"),
        },
        {
          id: "nav-settings",
          group: "navigate",
          title: t.nav.settings,
          hint: t.nav.settingsSub,
          keywords: "настройки settings",
          icon: <Settings className="h-4 w-4 text-ink3" />,
          run: () => onNavigateTab("settings"),
        },
      ];
      for (const s of strategies) {
        list.push({
          id: `strat-${s.id}`,
          group: "strategies",
          title: s.name,
          hint: s.engine === "winws" ? "Flowseal" : "zapret2 Lua",
          keywords: `${s.name} ${s.id}`.toLowerCase(),
          icon: <Zap className="h-4 w-4 text-accent" />,
          disabled: isTransitioning,
          run: () => onStartStrategy(s.id),
        });
      }
      return list;
    }, [
      t,
      isRunning,
      isTransitioning,
      isAutoTuning,
      isRefreshingProbes,
      isStrayCleaning,
      expertMode,
      strategies,
      onStart,
      onStop,
      onAutoTune,
      onRefreshProbes,
      onKillStray,
      onToggleExpert,
      onNavigateTab,
      onStartStrategy,
    ]);

    const q = query.trim().toLowerCase();
    const filtered = useMemo(() => {
      if (!q) return actions.filter((a) => a.group !== "strategies");
      return actions.filter(
        (a) =>
          a.title.toLowerCase().includes(q) ||
          a.hint.toLowerCase().includes(q) ||
          a.keywords.toLowerCase().includes(q)
      );
    }, [actions, q]);

    // Reset on open; clamp the selection when the list shrinks, and
    // never leave it on a disabled row (e.g. "start" while the
    // engine is transitioning): move to the nearest enabled row,
    // forward first.
    useEffect(() => {
      if (open) {
        setQuery("");
        setSelected(0);
        inputRef.current?.focus();
      }
    }, [open]);
    useEffect(() => {
      setSelected((s) => {
        if (filtered.length === 0) return 0;
        const clamped = Math.min(s, filtered.length - 1);
        if (!filtered[clamped]?.disabled) return clamped;
        for (let i = clamped + 1; i < filtered.length; i++) {
          if (!filtered[i].disabled) return i;
        }
        for (let i = clamped - 1; i >= 0; i--) {
          if (!filtered[i].disabled) return i;
        }
        return clamped;
      });
    }, [filtered]);

    // Keep the selected row visible while arrowing through the list.
    useEffect(() => {
      const el = listRef.current?.querySelector<HTMLElement>(
        `[data-palette-idx="${selected}"]`
      );
      el?.scrollIntoView({ block: "nearest" });
    }, [selected]);

    // Esc closes even when the input lost focus.
    useEffect(() => {
      if (!open) return;
      const onKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") onClose();
      };
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, [open, onClose]);

    if (!open) return null;

    const runSelected = (idx: number) => {
      const a = filtered[idx];
      if (!a || a.disabled) return;
      onClose();
      a.run();
    };

    // Arrow navigation skips disabled rows and stays put at the
    // ends (and on an empty list).
    const stepSelection = (dir: 1 | -1) => {
      setSelected((s) => {
        let next = s;
        for (let i = 0; i < filtered.length; i++) {
          next += dir;
          if (next < 0 || next >= filtered.length) return s;
          if (!filtered[next].disabled) return next;
        }
        return s;
      });
    };

    const onInputKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        stepSelection(1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        stepSelection(-1);
      } else if (e.key === "Enter") {
        e.preventDefault();
        runSelected(selected);
      }
    };

    const groupTitle = (g: PaletteAction["group"]) =>
      g === "actions"
        ? t.palette.groupActions
        : g === "navigate"
        ? t.palette.groupNavigate
        : t.palette.groupStrategies;

    let lastGroup: PaletteAction["group"] | null = null;

    return createPortal(
      <div
        className="fixed inset-0 z-50 flex items-start justify-center bg-overlay backdrop-blur-md p-4 modal-backdrop-enter"
        onClick={onClose}
      >
        <div
          className="modal-panel-enter mt-[10vh] w-full max-w-lg overflow-hidden rounded-panel border border-line bg-raise shadow-2xl"
          onClick={(e) => e.stopPropagation()}
          role="dialog"
          aria-label={t.palette.placeholder}
        >
          <div className="flex items-center gap-2.5 border-b border-line px-3.5 py-3">
            <Search className="h-4 w-4 shrink-0 text-accent" />
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setSelected(0);
              }}
              onKeyDown={onInputKeyDown}
              placeholder={t.palette.placeholder}
              className="min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink5 focus:outline-none"
            />
            <kbd className="shrink-0 rounded-control border border-line bg-cardin px-1.5 py-0.5 font-mono text-[10px] text-ink4">
              Esc
            </kbd>
          </div>
          <div ref={listRef} className="max-h-[320px] overflow-y-auto py-1.5">
            {filtered.length === 0 ? (
              <div className="px-3.5 py-6 text-center text-xs text-ink4">
                {t.palette.noResults}
              </div>
            ) : (
              filtered.map((a, idx) => {
                const header =
                  a.group !== lastGroup ? (
                    <div
                      key={`h-${a.group}`}
                      className="px-3.5 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-[0.14em] text-ink5"
                    >
                      {groupTitle(a.group)}
                    </div>
                  ) : null;
                lastGroup = a.group;
                return (
                  <React.Fragment key={a.id}>
                    {header}
                    <button
                      type="button"
                      data-palette-idx={idx}
                      disabled={a.disabled}
                      onClick={() => runSelected(idx)}
                      onMouseEnter={() => setSelected(idx)}
                      className={`flex w-full items-center gap-2.5 px-3.5 py-2 text-left transition-colors cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 ${
                        idx === selected ? "bg-accent-wash" : ""
                      }`}
                    >
                      <span className="shrink-0">{a.icon}</span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-xs font-semibold text-ink">
                          {a.title}
                        </span>
                        <span className="block truncate text-[11px] text-ink4">
                          {a.hint}
                        </span>
                      </span>
                    </button>
                  </React.Fragment>
                );
              })
            )}
          </div>
          <div className="border-t border-line px-3.5 py-2 text-[11px] text-ink5">
            {t.palette.hint}
          </div>
        </div>
      </div>,
      document.body
    );
  }
);
