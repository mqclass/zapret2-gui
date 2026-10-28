import React, { useState, useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  X,
  Search,
  Loader2,
  Check,
  AppWindow,
  ShieldCheck,
  ShieldOff,
  Shield,
  RefreshCw,
  AlertTriangle,
} from "lucide-react";
import { useI18n } from "../i18n";
import { isAutoTuneModalOpen, isProbeCycleRunning } from "../utils/watchdog";
import type {
  LuaStrategy,
  SupervisorStatus,
  SystemStatusResponse,
} from "../types";

interface AppInfo {
  pid: number;
  name: string;
  path: string;
}

interface AppSplitConfig {
  mode: string;
  apps: string[];
}

interface AppSplitModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const AppSplitModal: React.FC<AppSplitModalProps> = ({ isOpen, onClose }) => {
  const { t } = useI18n();
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [mode, setMode] = useState<"disabled" | "allowlist" | "blocklist">("disabled");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Supervisor snapshot for honest status notes and the restart decision.
  const [supervisor, setSupervisor] = useState<SupervisorStatus | null>(null);
  const [strategies, setStrategies] = useState<LuaStrategy[]>([]);
  // null = unknown (backend did not report binaries); false = split exe absent.
  const [splitBinaryPresent, setSplitBinaryPresent] = useState<boolean | null>(null);
  // Snapshot of the last saved config, for "did anything change?" checks.
  const [initialMode, setInitialMode] = useState<"disabled" | "allowlist" | "blocklist">("disabled");
  const [initialApps, setInitialApps] = useState<Set<string>>(new Set());
  const [confirmRestart, setConfirmRestart] = useState(false);
  const [deferredNote, setDeferredNote] = useState(false);
  const [customArgsNote, setCustomArgsNote] = useState(false);

  const loadData = async () => {
    setIsLoading(true);
    setError(null);
    setConfirmRestart(false);
    setDeferredNote(false);
    setCustomArgsNote(false);
    try {
      const [appList, config, systemStatus, strategyList] = await Promise.all([
        invoke<AppInfo[]>("list_running_apps"),
        invoke<AppSplitConfig>("get_app_split_config"),
        invoke<SystemStatusResponse>("get_system_status"),
        invoke<LuaStrategy[]>("list_strategies"),
      ]);
      setApps(appList);
      const validModes = ["disabled", "allowlist", "blocklist"] as const;
      const parsedMode = (validModes as readonly string[]).includes(config.mode)
        ? (config.mode as "disabled" | "allowlist" | "blocklist")
        : "disabled";
      setMode(parsedMode);
      const parsedApps = new Set(config.apps.map((p) => p.toLowerCase()));
      setSelected(parsedApps);
      setInitialMode(parsedMode);
      setInitialApps(parsedApps);
      setSupervisor(systemStatus.supervisor);
      setStrategies(strategyList);
      setSplitBinaryPresent(
        systemStatus.binaries
          ? systemStatus.binaries.winws2_split_exe != null
          : null
      );
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      loadData();
    }
  }, [isOpen]);

  const filtered = useMemo(() => {
    const q = search.toLowerCase().trim();
    if (!q) return apps;
    return apps.filter(
      (a) => a.name.toLowerCase().includes(q) || a.path.toLowerCase().includes(q)
    );
  }, [apps, search]);

  const toggleApp = (path: string) => {
    const key = path.toLowerCase();
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  };

  const engineRunning = supervisor?.state === "running";
  const activeStrategyId = supervisor?.active_strategy ?? null;
  const activeEngine = strategies.find((s) => s.id === activeStrategyId)?.engine ?? null;
  const splitEnforced = supervisor?.app_split_enforced === true;
  // Status notes: shown only for what is actually known. When the split
  // is enforced there is nothing to warn about. The Flowseal note needs
  // a running engine (only then is the active strategy known); the
  // binary note is static and holds while stopped too.
  const showFlowsealNote =
    !splitEnforced &&
    mode !== "disabled" &&
    (supervisor?.app_split_not_enforced_reason === "flowseal" ||
      (engineRunning && activeEngine === "winws"));
  const showBinaryMissingNote =
    !splitEnforced &&
    mode !== "disabled" &&
    (splitBinaryPresent === false ||
      supervisor?.app_split_not_enforced_reason === "binary_missing");
  // Empty allowlist means nothing goes through the bypass at all: every
  // app goes direct. Say so even when the split reports "enforced" —
  // enforced with an empty list is still a bypass that carries nothing.
  const showEmptyAllowlistNote = mode === "allowlist" && selected.size === 0;

  const setsEqual = (a: Set<string>, b: Set<string>) =>
    a.size === b.size && [...a].every((v) => b.has(v));
  const configChanged = mode !== initialMode || !setsEqual(selected, initialApps);

  const saveConfig = async () => {
    await invoke("set_app_split_config", { mode, apps: Array.from(selected) });
    setInitialMode(mode);
    setInitialApps(new Set(selected));
  };

  const handleApply = async () => {
    setIsSaving(true);
    setError(null);
    try {
      // Nothing changed: no save, no restart, just close.
      if (!configChanged) {
        onClose();
        return;
      }
      // Engine stopped: settings take effect at the next start, no
      // dialog and no restart to confirm.
      if (!engineRunning) {
        await saveConfig();
        onClose();
        return;
      }
      // A probe cycle owns the engine right now (manual auto-tune or a
      // watchdog escalation — both hold the same probe-cycle lock): a
      // restart here would interleave with it and corrupt its results.
      // Save only, say so plainly, and stay open so the note is seen.
      if (isProbeCycleRunning() || isAutoTuneModalOpen()) {
        await saveConfig();
        setDeferredNote(true);
        return;
      }
      // Engine running but started with one-off custom args
      // (active_strategy is null): restarting here would resolve the
      // null strategy to the flowseal-alt fallback and silently switch
      // strategies. Save only, say so plainly, and stay open so the
      // note is seen — same shape as the deferred branch above.
      if (engineRunning && activeStrategyId == null) {
        await saveConfig();
        setCustomArgsNote(true);
        return;
      }
      // Engine running and settings changed: ask first. Restarting
      // briefly drops the bypass — that must be said, not implied.
      if (!confirmRestart) {
        setConfirmRestart(true);
        return;
      }
      await saveConfig();
      await invoke("stop_supervisor");
      await invoke("start_supervisor", {
        strategyId: activeStrategyId ?? null,
        customArgs: null,
      });
      onClose();
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsSaving(false);
    }
  };

  const handleSaveOnly = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await saveConfig();
      onClose();
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsSaving(false);
    }
  };

  if (!isOpen) return null;

  // Portal to document.body: glass ancestors (backdrop-filter) would
  // otherwise become the containing block for `fixed` and confine the
  // backdrop to the card instead of the window (see CustomProfileModal).
  return createPortal(
    <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 modal-backdrop-enter">
      <div className="relative m-auto flex max-h-[90vh] w-full max-w-2xl flex-col rounded-card border border-raise bg-card shadow-2xl modal-panel-enter">
        {/* Header */}
        <div className="flex items-start justify-between border-b border-raise p-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-accent-wash border border-accent-line text-accent">
              <AppWindow className="h-5 w-5" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-ink">{t.appSplit.title}</h2>
              <p className="text-[11px] text-ink4">{t.appSplit.subtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-control p-1.5 text-ink4 hover:text-ink hover:bg-hovert transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Honest status notes: only what is actually known. Enforced
            split needs no note. The permanent limits below always apply. */}
        <div className="border-b border-raise px-5 pt-4 space-y-2 pb-4">
          {showFlowsealNote && (
            <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
              <span>{t.appSplit.noteFlowseal}</span>
            </div>
          )}
          {showBinaryMissingNote && (
            <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
              <span>{t.appSplit.noteBinaryMissing}</span>
            </div>
          )}
          {deferredNote && (
            <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
              <span>{t.appSplit.deferredNote}</span>
            </div>
          )}
          {customArgsNote && (
            <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
              <span>{t.appSplit.customArgsNote}</span>
            </div>
          )}
          {showEmptyAllowlistNote && (
            <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
              <span>{t.appSplit.noteEmptyAllowlist}</span>
            </div>
          )}
          <ul className="space-y-1 text-[11px] leading-relaxed text-ink4">
            <li>• {t.appSplit.honestBrowser}</li>
            <li>• {t.appSplit.honestUwpIpv6}</li>
            <li>• {t.appSplit.honestLaunchers}</li>
          </ul>
        </div>

        {/* Restart confirmation: shown only when the engine runs and the
            config actually changed. */}
        {confirmRestart && (
          <div className="border-b border-raise px-5 py-4">
            <div className="rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-3">
              <p className="text-xs font-semibold text-amber-300">
                {t.appSplit.confirmRestartTitle}
              </p>
              <p className="mt-1 text-[11px] leading-relaxed text-amber-300/90">
                {t.appSplit.restartWarning}
              </p>
              <div className="mt-3 flex gap-2">
                <button
                  onClick={handleApply}
                  disabled={isSaving}
                  className="flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold bg-accent-fill text-accent-contrast hover:opacity-90 transition-all cursor-pointer disabled:opacity-50"
                >
                  {isSaving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  <span>{t.appSplit.restartNow}</span>
                </button>
                <button
                  onClick={handleSaveOnly}
                  disabled={isSaving}
                  className="rounded-control px-3 py-1.5 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer disabled:opacity-50"
                >
                  {t.appSplit.saveOnly}
                </button>
                <button
                  onClick={() => setConfirmRestart(false)}
                  disabled={isSaving}
                  className="rounded-control px-3 py-1.5 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer disabled:opacity-50"
                >
                  {t.appSplit.cancel}
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Mode selector */}
        <div className="border-b border-raise p-4">
          <div className="grid grid-cols-3 gap-2">
            {(
              [
                { id: "disabled", icon: ShieldOff, label: t.appSplit.modeDisabled },
                { id: "allowlist", icon: ShieldCheck, label: t.appSplit.modeAllowlist },
                { id: "blocklist", icon: Shield, label: t.appSplit.modeBlocklist },
              ] as const
            ).map((m) => (
              <button
                key={m.id}
                onClick={() => setMode(m.id)}
                className={`flex flex-col items-center gap-1.5 rounded-panel border p-3 transition-all cursor-pointer ${
                  mode === m.id
                    ? "border-accent-line bg-accent-wash text-accent"
                    : "border-line bg-cardin text-ink4 hover:border-line hover:text-ink3"
                }`}
              >
                <m.icon className="h-4 w-4" />
                <span className="text-[11px] font-semibold">{m.label}</span>
              </button>
            ))}
          </div>
          <p className="mt-2 text-[11px] text-ink5 leading-tight">
            {mode === "disabled" && t.appSplit.modeDisabledDesc}
            {mode === "allowlist" && t.appSplit.modeAllowlistDesc}
            {mode === "blocklist" && t.appSplit.modeBlocklistDesc}
          </p>
        </div>

        {/* Search */}
        {mode !== "disabled" && (
          <div className="border-b border-raise p-3">
            <div className="relative">
              <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-ink5" />
              <input
                type="text"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t.appSplit.searchPlaceholder}
                className="w-full rounded-control border border-line bg-cardin pl-8 pr-2.5 py-1.5 text-xs text-ink placeholder:text-ink5 focus:border-accent-line focus:outline-none"
              />
            </div>
          </div>
        )}

        {/* App list */}
        {mode !== "disabled" && (
          <div className="flex-1 overflow-y-auto p-3 space-y-1 min-h-[200px]">
            {isLoading ? (
              <div className="flex items-center justify-center py-8">
                <Loader2 className="h-5 w-5 animate-spin text-accent" />
              </div>
            ) : filtered.length === 0 ? (
              <p className="text-center text-xs text-ink5 py-8">{t.appSplit.noApps}</p>
            ) : (
              filtered.map((app) => {
                const isSelected = selected.has(app.path.toLowerCase());
                return (
                  <button
                    key={`${app.pid}-${app.path}`}
                    onClick={() => toggleApp(app.path)}
                    className={`w-full flex items-center gap-3 rounded-panel border p-2.5 transition-all cursor-pointer text-left ${
                      isSelected
                        ? "border-accent-line bg-accent-wash"
                        : "border-line bg-cardin hover:border-line hover:bg-raise"
                    }`}
                  >
                    <div
                      className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-control border transition-all ${
                        isSelected
                          ? "border-accent-line bg-accent text-accent-contrast"
                          : "border-line text-transparent"
                      }`}
                    >
                      <Check className="h-3.5 w-3.5" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-xs font-medium text-ink truncate">{app.name}</p>
                      <p className="text-[10px] text-ink5 truncate font-mono">{app.path}</p>
                    </div>
                  </button>
                );
              })
            )}
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-raise p-4">
          <div className="flex items-center gap-2">
            <button
              onClick={loadData}
              disabled={isLoading}
              className="flex items-center gap-1.5 rounded-control px-2.5 py-1.5 text-xs text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer disabled:opacity-50"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? "animate-spin" : ""}`} />
              <span>{t.appSplit.refresh}</span>
            </button>
            {selected.size > 0 && mode !== "disabled" && (
              <span className="text-[11px] text-ink5">
                {t.appSplit.selectedCount.replace("{n}", String(selected.size))}
              </span>
            )}
          </div>
          <div className="flex gap-2">
            <button
              onClick={onClose}
              className="rounded-control px-3 py-1.5 text-xs font-medium text-ink4 hover:text-ink hover:bg-hovert border border-line transition-all cursor-pointer"
            >
              {t.appSplit.cancel}
            </button>
            <button
              onClick={handleApply}
              disabled={isSaving}
              className="flex items-center gap-1.5 rounded-control px-4 py-1.5 text-xs font-semibold bg-accent-fill text-accent-contrast hover:opacity-90 transition-all cursor-pointer disabled:opacity-50"
            >
              {isSaving && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              <span>{confirmRestart ? t.appSplit.restartNow : t.appSplit.apply}</span>
            </button>
          </div>
        </div>

        {error && (
          <div className="border-t border-raise p-3">
            <p className="text-xs text-rose-400">{error}</p>
          </div>
        )}
      </div>
    </div>,
    document.body
  );
};
