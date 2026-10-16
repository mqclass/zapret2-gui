import React from "react";
import {
  ShieldCheck,
  ShieldAlert,
  RefreshCw,
  Trash2,
  Globe,
  Cpu,
  Rocket,
  Wrench,
  Globe2,
  Tv,
  CloudDownload,
} from "lucide-react";
import { useI18n } from "../i18n";
import { Language } from "../i18n/types";

interface HeaderProps {
  isElevated: boolean;
  onRelaunchElevated: () => void;
  onKillStray: () => void;
  isStrayCleaning: boolean;
  isAutostart: boolean;
  onToggleAutostart: () => void;
  isTogglingAutostart: boolean;
  onOpenDomainManager: () => void;
  onOpenDoctor: () => void;
  onOpenDns: () => void;
  onOpenSpeedtest: () => void;
  onSyncLists: () => void;
  isSyncingLists: boolean;
}

export const Header: React.FC<HeaderProps> = React.memo(
  ({
    isElevated,
    onRelaunchElevated,
    onKillStray,
    isStrayCleaning,
    isAutostart,
    onToggleAutostart,
    isTogglingAutostart,
    onOpenDomainManager,
    onOpenDoctor,
    onOpenDns,
    onOpenSpeedtest,
    onSyncLists,
    isSyncingLists,
  }) => {
    const { language, setLanguage, t } = useI18n();

    const languages: { code: Language; label: string }[] = [
      { code: "ru", label: "RU" },
      { code: "en", label: "EN" },
      { code: "uk", label: "UK" },
    ];

    return (
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-raise bg-card/60 px-6 py-3.5 backdrop-blur-md">
        {/* Brand / Logo */}
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-panel bg-gradient-to-tr from-accent-fill to-accent-fill-hover shadow-lg shadow-accent/20">
            <Cpu className="h-5 w-5 text-ink" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-lg font-bold tracking-tight text-ink">{t.header.title}</h1>
              <span className="rounded-chip bg-accent-wash px-2 py-0.5 text-xs font-semibold text-accent border border-accent-line font-mono">
                {t.header.version}
              </span>
            </div>
            <p className="text-xs text-ink4">{t.header.subtitle}</p>
          </div>
        </div>

        {/* Right Action Bar */}
        <div className="flex flex-wrap items-center gap-3">
          {/* Language Switcher */}
          <div className="flex items-center gap-1 rounded-panel border border-raise bg-cardin/80 p-1">
            <Globe className="h-3.5 w-3.5 ml-1.5 text-ink5 mr-0.5" />
            {languages.map((l) => (
              <button
                key={l.code}
                onClick={() => setLanguage(l.code)}
                className={`rounded-control px-2.5 py-1 text-xs font-semibold transition-all cursor-pointer ${
                  language === l.code
                    ? "bg-accent-fill text-ink shadow-sm"
                    : "text-ink4 hover:text-ink2 hover:bg-raise/60"
                }`}
                title={
                  l.code === "ru"
                    ? t.header.langRu
                    : l.code === "en"
                    ? t.header.langEn
                    : t.header.langUk
                }
              >
                {l.label}
              </button>
            ))}
          </div>

          {/* Autostart with Windows Toggle */}
          <button
            onClick={onToggleAutostart}
            disabled={isTogglingAutostart}
            className={`flex items-center gap-2 rounded-panel border px-3 py-1.5 text-xs font-medium transition-all cursor-pointer disabled:opacity-60 ${
              isAutostart
                ? "bg-indigo-600/15 border-indigo-500/40 text-indigo-300 hover:bg-indigo-600/25 shadow-sm shadow-indigo-500/10"
                : "bg-cardin/80 border-raise text-ink4 hover:text-ink2 hover:bg-raise/60"
            }`}
            title={
              isAutostart
                ? t.header.autostartEnabledTooltip
                : t.header.autostartDisabledTooltip
            }
          >
            {isTogglingAutostart ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-indigo-400" />
            ) : (
              <Rocket
                className={`h-3.5 w-3.5 ${
                  isAutostart ? "text-indigo-400" : "text-ink5"
                }`}
              />
            )}
            <span>{t.header.autostart}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isAutostart ? "bg-indigo-400 shadow-[0_0_6px_rgb(var(--c-accent))]" : "bg-ink5"
              }`}
            />
          </button>

          {/* Auto-Doctor Modal Trigger */}
          <button
            onClick={onOpenDoctor}
            className="flex items-center gap-1.5 rounded-panel border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-xs font-semibold text-amber-300 hover:bg-amber-500/20 hover:border-amber-500/50 shadow-sm shadow-amber-500/10 transition-all cursor-pointer"
            title={t.doctor.title}
          >
            <Wrench className="h-3.5 w-3.5 text-amber-400" />
            <span>Доктор</span>
          </button>

          {/* DNS Manager Modal Trigger */}
          <button
            onClick={onOpenDns}
            className="flex items-center gap-1.5 rounded-panel border border-cyan-500/30 bg-cyan-500/10 px-3 py-1.5 text-xs font-semibold text-cyan-300 hover:bg-cyan-500/20 hover:border-cyan-500/50 shadow-sm shadow-cyan-500/10 transition-all cursor-pointer"
            title={t.dns.title}
          >
            <Globe2 className="h-3.5 w-3.5 text-cyan-400" />
            <span>DNS</span>
          </button>

          {/* YouTube 4K Speedtest Modal Trigger */}
          <button
            onClick={onOpenSpeedtest}
            className="flex items-center gap-1.5 rounded-panel border border-rose-500/30 bg-rose-500/10 px-3 py-1.5 text-xs font-semibold text-rose-300 hover:bg-rose-500/20 hover:border-rose-500/50 shadow-sm shadow-rose-500/10 transition-all cursor-pointer"
            title={t.speedtest.title}
          >
            <Tv className="h-3.5 w-3.5 text-rose-400" />
            <span>Тест 4K</span>
          </button>

          {/* Remote Lists Sync Button */}
          <button
            onClick={onSyncLists}
            disabled={isSyncingLists}
            className="flex items-center gap-1.5 rounded-panel border border-indigo-500/30 bg-indigo-500/10 px-3 py-1.5 text-xs font-semibold text-indigo-300 hover:bg-indigo-500/20 hover:border-indigo-500/50 shadow-sm shadow-indigo-500/10 transition-all cursor-pointer disabled:opacity-50"
            title={t.syncLists.syncBtn}
          >
            <CloudDownload className={`h-3.5 w-3.5 text-indigo-400 ${isSyncingLists ? "animate-bounce" : ""}`} />
            <span>{isSyncingLists ? t.syncLists.syncing : t.syncLists.syncBtn}</span>
          </button>

          {/* Domain & Sites Manager Modal Trigger */}
          <button
            onClick={onOpenDomainManager}
            className="flex items-center gap-2 rounded-panel border border-emerald-500/30 bg-emerald-500/10 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/20 hover:border-emerald-500/50 shadow-sm shadow-emerald-500/10 transition-all cursor-pointer"
            title={t.domainManager.title}
          >
            <Globe className="h-3.5 w-3.5 text-emerald-400" />
            <span>{t.header.sitesManager}</span>
          </button>

          {/* Privilege Elevation Pill */}
          {isElevated ? (
            <div className="flex items-center gap-1.5 rounded-full bg-emerald-500/10 border border-emerald-500/20 px-3 py-1 text-xs font-medium text-emerald-400">
              <ShieldCheck className="h-3.5 w-3.5 text-emerald-400" />
              <span>{t.header.adminReady}</span>
            </div>
          ) : (
            <button
              onClick={onRelaunchElevated}
              className="flex items-center gap-1.5 rounded-full bg-amber-500/15 border border-amber-500/30 px-3 py-1 text-xs font-medium text-amber-400 hover:bg-amber-500/25 transition-all cursor-pointer"
              title={t.header.adminRequired}
            >
              <ShieldAlert className="h-3.5 w-3.5 text-amber-400" />
              <span>{t.header.adminRequired}</span>
            </button>
          )}

          {/* Stray process cleanup button */}
          <button
            onClick={onKillStray}
            disabled={isStrayCleaning}
            className="flex items-center gap-1.5 rounded-panel border border-linestrong/60 bg-raise/80 px-3 py-1.5 text-xs font-medium text-ink3 hover:bg-linestrong hover:text-ink transition-all disabled:opacity-50 cursor-pointer"
            title={t.header.cleanProcessesTooltip}
          >
            {isStrayCleaning ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-ink4" />
            ) : (
              <Trash2 className="h-3.5 w-3.5 text-ink4" />
            )}
            <span>{t.header.cleanProcesses}</span>
          </button>
        </div>
      </header>
    );
  }
);
