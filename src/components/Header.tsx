import React from "react";
import { ShieldCheck, ShieldAlert, RefreshCw, Trash2, Globe, Cpu, Rocket } from "lucide-react";
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
  }) => {
    const { language, setLanguage, t } = useI18n();

    const languages: { code: Language; label: string }[] = [
      { code: "ru", label: "RU" },
      { code: "en", label: "EN" },
      { code: "uk", label: "UK" },
    ];

    return (
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-slate-800 bg-slate-900/60 px-6 py-3.5 backdrop-blur-md">
        {/* Brand / Logo */}
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-600 shadow-lg shadow-blue-500/20">
            <Cpu className="h-5 w-5 text-white" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-lg font-bold tracking-tight text-white">{t.header.title}</h1>
              <span className="rounded-md bg-blue-500/10 px-2 py-0.5 text-xs font-semibold text-blue-400 border border-blue-500/20 font-mono">
                {t.header.version}
              </span>
            </div>
            <p className="text-xs text-slate-400">{t.header.subtitle}</p>
          </div>
        </div>

        {/* Right Action Bar */}
        <div className="flex flex-wrap items-center gap-3">
          {/* Language Switcher */}
          <div className="flex items-center gap-1 rounded-xl border border-slate-800 bg-slate-950/80 p-1">
            <Globe className="h-3.5 w-3.5 ml-1.5 text-slate-500 mr-0.5" />
            {languages.map((l) => (
              <button
                key={l.code}
                onClick={() => setLanguage(l.code)}
                className={`rounded-lg px-2.5 py-1 text-xs font-semibold transition-all cursor-pointer ${
                  language === l.code
                    ? "bg-blue-600 text-white shadow-sm"
                    : "text-slate-400 hover:text-slate-200 hover:bg-slate-800/60"
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
            className={`flex items-center gap-2 rounded-xl border px-3 py-1.5 text-xs font-medium transition-all cursor-pointer disabled:opacity-60 ${
              isAutostart
                ? "bg-indigo-600/15 border-indigo-500/40 text-indigo-300 hover:bg-indigo-600/25 shadow-sm shadow-indigo-500/10"
                : "bg-slate-950/80 border-slate-800 text-slate-400 hover:text-slate-200 hover:bg-slate-800/60"
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
                  isAutostart ? "text-indigo-400" : "text-slate-500"
                }`}
              />
            )}
            <span>{t.header.autostart}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isAutostart ? "bg-indigo-400 shadow-[0_0_6px_#818cf8]" : "bg-slate-600"
              }`}
            />
          </button>

          {/* Domain & Sites Manager Modal Trigger */}
          <button
            onClick={onOpenDomainManager}
            className="flex items-center gap-2 rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-3 py-1.5 text-xs font-semibold text-emerald-300 hover:bg-emerald-500/20 hover:border-emerald-500/50 shadow-sm shadow-emerald-500/10 transition-all cursor-pointer"
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
            className="flex items-center gap-1.5 rounded-xl border border-slate-700/60 bg-slate-800/80 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-700 hover:text-white transition-all disabled:opacity-50 cursor-pointer"
            title={t.header.cleanProcessesTooltip}
          >
            {isStrayCleaning ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-slate-400" />
            ) : (
              <Trash2 className="h-3.5 w-3.5 text-slate-400" />
            )}
            <span>{t.header.cleanProcesses}</span>
          </button>
        </div>
      </header>
    );
  }
);
