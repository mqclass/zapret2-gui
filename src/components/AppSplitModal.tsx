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

  const loadData = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const [appList, config] = await Promise.all([
        invoke<AppInfo[]>("list_running_apps"),
        invoke<AppSplitConfig>("get_app_split_config"),
      ]);
      setApps(appList);
      const validModes = ["disabled", "allowlist", "blocklist"] as const;
      setMode(
        (validModes as readonly string[]).includes(config.mode)
          ? (config.mode as "disabled" | "allowlist" | "blocklist")
          : "disabled"
      );
      setSelected(new Set(config.apps.map((p) => p.toLowerCase())));
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

  const handleApply = async () => {
    setIsSaving(true);
    setError(null);
    try {
      await invoke("set_app_split_config", { mode, apps: Array.from(selected) });
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

        {/* Honest status: config persists, traffic filtering is not wired yet */}
        <div className="border-b border-raise px-5 pt-4">
          <div className="flex items-start gap-2 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-[11px] leading-relaxed text-amber-300">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" />
            <span>{t.appSplit.devNotice}</span>
          </div>
        </div>

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
              <span>{t.appSplit.apply}</span>
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
