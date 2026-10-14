import React from "react";
import {
  Rocket,
  ShieldCheck,
  ShieldAlert,
  Trash2,
  AlertTriangle,
  RefreshCw,
  Globe,
  ExternalLink,
  Shield,
} from "lucide-react";
import { ZapretLogo } from "../ZapretLogo";
import { useI18n } from "../../i18n";
import { Language } from "../../i18n/types";

interface SettingsViewProps {
  isElevated: boolean;
  onRelaunchElevated: () => void;
  isAutostart: boolean;
  onToggleAutostart: () => void;
  isTogglingAutostart: boolean;
  onKillStray: () => void;
  isStrayCleaning: boolean;
  onResolveConflicts: () => void;
  isResolvingConflicts: boolean;
}

export const SettingsView: React.FC<SettingsViewProps> = React.memo(
  ({
    isElevated,
    onRelaunchElevated,
    isAutostart,
    onToggleAutostart,
    isTogglingAutostart,
    onKillStray,
    isStrayCleaning,
    onResolveConflicts,
    isResolvingConflicts,
  }) => {
    const { language, setLanguage, t } = useI18n();

    const languages: { code: Language; label: string; full: string }[] = [
      { code: "ru", label: "RU", full: "Русский" },
      { code: "en", label: "EN", full: "English" },
      { code: "uk", label: "UK", full: "Українська" },
    ];

    return (
      <div className="space-y-4 pb-4 max-w-4xl">
        {/* System & Engine Section */}
        <div className="rounded-2xl border border-[#212635] bg-[#141720] p-5 space-y-4">
          <div className="border-b border-[#202534] pb-3">
            <h3 className="text-xs font-semibold text-white flex items-center gap-2">
              <Shield className="h-4 w-4 text-blue-400" />
              <span>{t.settingsTab.systemSection}</span>
            </h3>
          </div>

          <div className="space-y-2.5">
            {/* Autostart Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-[#202534] bg-[#12151d] p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <Rocket className="h-4 w-4 text-blue-400" />
                  <span className="text-xs font-semibold text-white">Автозапуск при входе в Windows</span>
                  <span
                    className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
                      isAutostart
                        ? "bg-blue-500/15 text-blue-300 border border-blue-500/30"
                        : "bg-[#1d222e] text-slate-400"
                    }`}
                  >
                    {isAutostart ? "Включен" : "Отключен"}
                  </span>
                </div>
                <p className="text-[11px] text-slate-400 max-w-lg leading-relaxed">
                  {t.settingsTab.autostartDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onToggleAutostart}
                disabled={isTogglingAutostart}
                className={`flex items-center justify-center gap-2 rounded-lg px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 ${
                  isAutostart
                    ? "bg-blue-600/15 hover:bg-blue-600/25 text-blue-300 border border-blue-500/30"
                    : "bg-[#181d28] hover:bg-[#202636] text-slate-200 border border-[#2b3345]"
                }`}
              >
                {isTogglingAutostart ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-blue-400" />
                ) : (
                  <Rocket className="h-3.5 w-3.5 text-blue-400" />
                )}
                <span>{isAutostart ? "Отключить автозапуск" : "Включить автозапуск"}</span>
              </button>
            </div>

            {/* Elevation Status Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-[#202534] bg-[#12151d] p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  {isElevated ? (
                    <ShieldCheck className="h-4 w-4 text-emerald-400" />
                  ) : (
                    <ShieldAlert className="h-4 w-4 text-amber-400" />
                  )}
                  <span className="text-xs font-semibold text-white">{t.settingsTab.elevationTitle}</span>
                  <span
                    className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
                      isElevated
                        ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                        : "bg-amber-500/15 text-amber-400 border border-amber-500/30"
                    }`}
                  >
                    {isElevated ? "Администратор" : "Ограниченный"}
                  </span>
                </div>
                <p className="text-[11px] text-slate-400 max-w-lg leading-relaxed">
                  {t.settingsTab.elevationDesc}
                </p>
              </div>

              {!isElevated && (
                <button
                  type="button"
                  onClick={onRelaunchElevated}
                  className="flex items-center justify-center gap-2 rounded-lg bg-amber-500 hover:bg-amber-400 text-slate-950 px-3.5 py-2 text-xs font-semibold transition-all active:scale-[0.98] cursor-pointer shrink-0"
                >
                  <ShieldAlert className="h-3.5 w-3.5" />
                  <span>{t.settingsTab.elevationBtn}</span>
                </button>
              )}
            </div>

            {/* Stray Processes Cleanup Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-[#202534] bg-[#12151d] p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <Trash2 className="h-4 w-4 text-rose-400" />
                  <span className="text-xs font-semibold text-white">{t.settingsTab.strayTitle}</span>
                </div>
                <p className="text-[11px] text-slate-400 max-w-lg leading-relaxed">
                  {t.settingsTab.strayDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onKillStray}
                disabled={isStrayCleaning}
                className="flex items-center justify-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 hover:bg-rose-500/20 px-3.5 py-2 text-xs font-semibold text-rose-300 transition-all cursor-pointer disabled:opacity-50 shrink-0"
              >
                <Trash2 className={`h-3.5 w-3.5 ${isStrayCleaning ? "animate-spin" : ""}`} />
                <span>{t.settingsTab.strayBtn}</span>
              </button>
            </div>

            {/* Conflict Resolver Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-[#202534] bg-[#12151d] p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <AlertTriangle className="h-4 w-4 text-amber-400" />
                  <span className="text-xs font-semibold text-white">{t.settingsTab.conflictsTitle}</span>
                </div>
                <p className="text-[11px] text-slate-400 max-w-lg leading-relaxed">
                  {t.settingsTab.conflictsDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onResolveConflicts}
                disabled={isResolvingConflicts}
                className="flex items-center justify-center gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 hover:bg-amber-500/20 px-3.5 py-2 text-xs font-semibold text-amber-300 transition-all cursor-pointer disabled:opacity-50 shrink-0"
              >
                <AlertTriangle className={`h-3.5 w-3.5 ${isResolvingConflicts ? "animate-spin" : ""}`} />
                <span>{t.settingsTab.conflictsBtn}</span>
              </button>
            </div>
          </div>
        </div>

        {/* Language Selection Card */}
        <div className="rounded-2xl border border-[#212635] bg-[#141720] p-5 space-y-3">
          <div className="border-b border-[#202534] pb-3">
            <h3 className="text-xs font-semibold text-white flex items-center gap-2">
              <Globe className="h-4 w-4 text-blue-400" />
              <span>{t.settingsTab.languageSection}</span>
            </h3>
          </div>

          <div className="grid grid-cols-3 gap-2.5">
            {languages.map((item) => {
              const isSelected = language === item.code;
              return (
                <button
                  type="button"
                  key={item.code}
                  onClick={() => setLanguage(item.code)}
                  className={`flex items-center justify-between p-3 rounded-xl border text-xs font-medium transition-all cursor-pointer ${
                    isSelected
                      ? "border-[#384359] bg-[#1d2332] text-white shadow-sm"
                      : "border-[#202534] bg-[#12151d] text-slate-400 hover:text-slate-200 hover:bg-[#161a23]"
                  }`}
                >
                  <span className="font-semibold">{item.full}</span>
                  <span className="font-mono text-[10px] text-slate-500">{item.label}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Security & Warning Against Fakes */}
        <div className="rounded-2xl border border-[#2d261a] bg-[#161411] p-4 space-y-2">
          <div className="flex items-center gap-2 text-amber-400">
            <AlertTriangle className="h-4 w-4" />
            <h4 className="text-xs font-semibold text-white">{t.settingsTab.warningFakesTitle}</h4>
          </div>
          <p className="text-[11px] text-slate-300 leading-relaxed">
            {t.settingsTab.warningFakesBody}
          </p>
          <div className="flex flex-wrap gap-4 pt-1 text-xs">
            <a
              href="https://github.com/bol-van/zapret"
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 text-blue-400 hover:text-blue-300 hover:underline"
            >
              <span>GitHub bol-van/zapret</span>
              <ExternalLink className="h-3 w-3" />
            </a>
            <a
              href="https://github.com/Flowseal/zapret-discord-youtube"
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 text-blue-400 hover:text-blue-300 hover:underline"
            >
              <span>GitHub Flowseal (Пресеты)</span>
              <ExternalLink className="h-3 w-3" />
            </a>
            <a
              href="https://github.com/mqclass/zapret2-gui"
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-1.5 text-blue-400 hover:text-blue-300 hover:underline"
            >
              <span>GitHub mqclass/zapret2-gui</span>
              <ExternalLink className="h-3 w-3" />
            </a>
          </div>
        </div>

        {/* Application Architecture & Versions */}
        <div className="rounded-2xl border border-[#212635] bg-[#141720] p-4">
          <div className="flex items-center justify-between text-xs text-slate-400">
            <div className="flex items-center gap-2">
              <ZapretLogo className="h-4 w-4" />
              <span className="font-semibold text-white">zapret2</span>
              <span className="font-mono text-blue-400 font-semibold">{t.header.version}</span>
            </div>
            <div className="font-mono text-[11px] text-slate-500">
              WinWS v2.1.8 • WinDivert v2.2.2-F • Tauri v2
            </div>
          </div>
        </div>
      </div>
    );
  }
);
