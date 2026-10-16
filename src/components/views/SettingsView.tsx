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
  Palette,
  Check,
  Bell,
  BellOff,
  FileDown,
  ArrowDownToLine,
  BadgeCheck,
} from "lucide-react";
import { ZapretLogo } from "../ZapretLogo";
import { useI18n } from "../../i18n";
import { Language } from "../../i18n/types";
import { useTheme, ThemeId } from "../../theme/ThemeContext";
import { openUrl } from "@tauri-apps/plugin-opener";
import { UpdateInfo } from "../../types";

/** External links opened via the opener plugin (L-2). The capability allowlist
 *  restricts this to https://github.com/* — no webview navigation is enabled. */
const EXTERNAL_LINKS = [
  { url: "https://github.com/bol-van/zapret", label: "GitHub bol-van/zapret" },
  { url: "https://github.com/Flowseal/zapret-discord-youtube", label: "GitHub Flowseal (Пресеты)" },
  { url: "https://github.com/mqclass/zapret2-gui", label: "GitHub mqclass/zapret2-gui" },
];

/** Static miniature mock of each theme — pure CSS, no app state involved. */
const ThemePreview: React.FC<{ id: ThemeId }> = ({ id }) => {
  const isMacos = id === "macos";
  const bar = isMacos ? "#0f141d" : "#090c10";
  const line = isMacos ? "#232c40" : "#1a222b";
  const cardBg = isMacos ? "rgba(19,25,36,.92)" : "#0b0f14";
  const accent = isMacos ? "#0a84ff" : "#a3e635";
  const fill = isMacos ? "#0a84ff" : "#4d7c0f";
  const r = isMacos ? 8 : 3;
  return (
    <div
      className="theme-preview h-[76px] w-full"
      style={{ background: isMacos ? "#0a0d13" : "#05070a", borderRadius: r }}
      aria-hidden
    >
      <div className="flex h-full">
        <div
          className="flex flex-col gap-1 p-1.5"
          style={{ width: 30, background: bar, borderRight: `1px solid ${line}` }}
        >
          <div style={{ height: 6, borderRadius: 3, background: accent, opacity: 0.95 }} />
          <div style={{ height: 4, borderRadius: 2, background: line }} />
          <div style={{ height: 4, borderRadius: 2, background: line }} />
          <div style={{ height: 4, borderRadius: 2, background: line, opacity: 0.55 }} />
        </div>
        <div className="flex-1 space-y-1.5 p-1.5">
          <div
            style={{ height: 16, borderRadius: r - 2, background: cardBg, border: `1px solid ${line}` }}
          />
          <div className="flex gap-1.5">
            <div style={{ flex: 1, height: 11, borderRadius: r - 2, background: fill }} />
            <div
              style={{
                flex: 1,
                height: 11,
                borderRadius: r - 2,
                background: "transparent",
                border: `1px solid ${isMacos ? "#35415e" : "#2c3947"}`,
              }}
            />
          </div>
        </div>
      </div>
    </div>
  );
};

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
  notificationsOn: boolean;
  onToggleNotifications: () => void;
  isTogglingNotifications: boolean;
  onExportDiagnostics: () => void;
  isExportingDiag: boolean;
  autoUpdateOn: boolean;
  onToggleAutoUpdate: () => void;
  isTogglingAutoUpdate: boolean;
  onCheckUpdates: () => void;
  isCheckingUpdates: boolean;
  updateInfo: UpdateInfo | null;
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
    notificationsOn,
    onToggleNotifications,
    isTogglingNotifications,
    onExportDiagnostics,
    isExportingDiag,
    autoUpdateOn,
    onToggleAutoUpdate,
    isTogglingAutoUpdate,
    onCheckUpdates,
    isCheckingUpdates,
    updateInfo,
  }) => {
    const { language, setLanguage, t } = useI18n();
    const { theme, setTheme } = useTheme();

    const languages: { code: Language; label: string; full: string }[] = [
      { code: "ru", label: "RU", full: "Русский" },
      { code: "en", label: "EN", full: "English" },
      { code: "uk", label: "UK", full: "Українська" },
    ];

    const themes: { id: ThemeId; name: string; desc: string }[] = [
      {
        id: "macos",
        name: t.settingsTab.themeMacosName,
        desc: t.settingsTab.themeMacosDesc,
      },
      {
        id: "hyprland",
        name: t.settingsTab.themeHyprlandName,
        desc: t.settingsTab.themeHyprlandDesc,
      },
    ];

    return (
      <div className="space-y-4 pb-4 max-w-4xl">
        {/* System & Engine Section */}
        <div className="rounded-card border border-line bg-card p-5 space-y-4">
          <div className="border-b border-line pb-3">
            <h3 className="t-section-title">
              <Shield className="h-4 w-4 text-accent" />
              <span>{t.settingsTab.systemSection}</span>
            </h3>
          </div>

          <div className="space-y-2.5">
            {/* Autostart Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <Rocket className="h-4 w-4 text-accent" />
                  <span className="text-xs font-semibold text-ink">Автозапуск при входе в Windows</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      isAutostart
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {isAutostart ? "Включен" : "Отключен"}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.autostartDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onToggleAutostart}
                disabled={isTogglingAutostart}
                className={`flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 ${
                  isAutostart
                    ? "bg-accent-wash hover:bg-accent-wash text-accent-soft border border-accent-line"
                    : "bg-hovert hover:bg-hovert text-ink2 border border-line"
                }`}
              >
                {isTogglingAutostart ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : (
                  <Rocket className="h-3.5 w-3.5 text-accent" />
                )}
                <span>{isAutostart ? "Отключить автозапуск" : "Включить автозапуск"}</span>
              </button>
            </div>

            {/* Elevation Status Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  {isElevated ? (
                    <ShieldCheck className="h-4 w-4 text-emerald-400" />
                  ) : (
                    <ShieldAlert className="h-4 w-4 text-amber-400" />
                  )}
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.elevationTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      isElevated
                        ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                        : "bg-amber-500/15 text-amber-400 border border-amber-500/30"
                    }`}
                  >
                    {isElevated ? "Администратор" : "Ограниченный"}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.elevationDesc}
                </p>
              </div>

              {!isElevated && (
                <button
                  type="button"
                  onClick={onRelaunchElevated}
                  className="flex items-center justify-center gap-2 rounded-control bg-amber-500 hover:bg-amber-400 text-slate-950 px-3.5 py-2 text-xs font-semibold transition-all active:scale-[0.98] cursor-pointer shrink-0"
                >
                  <ShieldAlert className="h-3.5 w-3.5" />
                  <span>{t.settingsTab.elevationBtn}</span>
                </button>
              )}
            </div>

            {/* Stray Processes Cleanup Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <Trash2 className="h-4 w-4 text-rose-400" />
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.strayTitle}</span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.strayDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onKillStray}
                disabled={isStrayCleaning}
                className="flex items-center justify-center gap-2 rounded-control border border-rose-500/30 bg-rose-500/10 hover:bg-rose-500/20 px-3.5 py-2 text-xs font-semibold text-rose-300 transition-all cursor-pointer disabled:opacity-50 shrink-0"
              >
                <Trash2 className={`h-3.5 w-3.5 ${isStrayCleaning ? "animate-spin" : ""}`} />
                <span>{t.settingsTab.strayBtn}</span>
              </button>
            </div>

            {/* Conflict Resolver Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <AlertTriangle className="h-4 w-4 text-amber-400" />
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.conflictsTitle}</span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.conflictsDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onResolveConflicts}
                disabled={isResolvingConflicts}
                className="flex items-center justify-center gap-2 rounded-control border border-amber-500/30 bg-amber-500/10 hover:bg-amber-500/20 px-3.5 py-2 text-xs font-semibold text-amber-300 transition-all cursor-pointer disabled:opacity-50 shrink-0"
              >
                <AlertTriangle className={`h-3.5 w-3.5 ${isResolvingConflicts ? "animate-spin" : ""}`} />
                <span>{t.settingsTab.conflictsBtn}</span>
              </button>
            </div>

            {/* Windows Notifications Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  {notificationsOn ? (
                    <Bell className="h-4 w-4 text-accent" />
                  ) : (
                    <BellOff className="h-4 w-4 text-ink4" />
                  )}
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.notificationsTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      notificationsOn
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {notificationsOn ? t.settingsTab.notificationsOn : t.settingsTab.notificationsOff}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.notificationsDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onToggleNotifications}
                disabled={isTogglingNotifications}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isTogglingNotifications ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : notificationsOn ? (
                  <BellOff className="h-3.5 w-3.5 text-accent" />
                ) : (
                  <Bell className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {notificationsOn
                    ? t.settingsTab.notificationsBtnDisable
                    : t.settingsTab.notificationsBtnEnable}
                </span>
              </button>
            </div>

            {/* Application Updates Subsection */}
            <div className="flex items-center gap-2 pt-2">
              <ArrowDownToLine className="h-4 w-4 text-accent" />
              <span className="text-xs font-semibold text-ink">{t.update.sectionTitle}</span>
            </div>
            <p className="-mt-1 text-[11px] text-ink4 leading-relaxed">{t.update.sectionDesc}</p>

            {/* Auto-Update Check Toggle Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  {autoUpdateOn ? (
                    <RefreshCw className="h-4 w-4 text-accent" />
                  ) : (
                    <RefreshCw className="h-4 w-4 text-ink4" />
                  )}
                  <span className="text-xs font-semibold text-ink">{t.update.autoCheckTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      autoUpdateOn
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {autoUpdateOn ? t.update.autoCheckOn : t.update.autoCheckOff}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.update.autoCheckDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onToggleAutoUpdate}
                disabled={isTogglingAutoUpdate}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isTogglingAutoUpdate ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : autoUpdateOn ? (
                  <BellOff className="h-3.5 w-3.5 text-accent" />
                ) : (
                  <Bell className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {autoUpdateOn ? t.update.autoCheckOff : t.update.autoCheckOn}
                </span>
              </button>
            </div>

            {/* Version & Manual Check Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  {updateInfo?.update_available ? (
                    <AlertTriangle className="h-4 w-4 text-amber-400" />
                  ) : (
                    <BadgeCheck className="h-4 w-4 text-accent" />
                  )}
                  <span className="text-xs font-semibold text-ink">{t.update.versionTitle}</span>
                  <span className="rounded-chip px-1.5 py-0.5 text-[10px] font-mono bg-hovert text-ink4">
                    v{updateInfo?.current_version ?? t.header.version.replace(/^v/, "")}                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {updateInfo
                    ? updateInfo.update_available
                      ? t.update.availableTitle.replace("{v}", updateInfo.latest_version)
                      : t.update.upToDate
                    : t.update.versionDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onCheckUpdates}
                disabled={isCheckingUpdates}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 text-accent ${isCheckingUpdates ? "animate-spin" : ""}`}
                />
                <span>{isCheckingUpdates ? t.update.checking : t.update.checkBtn}</span>
              </button>
            </div>

            {/* Diagnostic Bundle Export Card */}            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <FileDown className="h-4 w-4 text-accent" />
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.exportDiagTitle}</span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.exportDiagDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onExportDiagnostics}
                disabled={isExportingDiag}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isExportingDiag ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : (
                  <FileDown className="h-3.5 w-3.5 text-accent" />
                )}
                <span>{t.settingsTab.exportDiagBtn}</span>
              </button>
            </div>
          </div>
        </div>

        {/* Theme Selection Card */}
        <div className="rounded-card border border-line bg-card p-5 space-y-3 shadow-card">
          <div className="border-b border-line pb-3">
            <h3 className="t-section-title">
              <Palette className="h-4 w-4 text-accent" />
              <span>{t.settingsTab.themeSection}</span>
            </h3>
          </div>

          <div className="grid grid-cols-2 gap-2.5">
            {themes.map((item) => {
              const isSelected = theme === item.id;
              return (
                <button
                  type="button"
                  key={item.id}
                  onClick={() => setTheme(item.id)}
                  aria-pressed={isSelected}
                  className={`group space-y-2.5 rounded-panel border p-3 text-left transition-all cursor-pointer ${
                    isSelected
                      ? "border-accent-line bg-accent-wash shadow-glow"
                      : "border-line bg-cardin hover:border-linestrong hover:bg-raise"
                  }`}
                >
                  <ThemePreview id={item.id} />
                  <div className="space-y-0.5">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-semibold text-ink">
                        {item.name}
                      </span>
                      {isSelected && (
                        <span className="t-badge t-badge-accent">
                          <Check className="h-3 w-3" />
                        </span>
                      )}
                    </div>
                    <p className="text-[11px] leading-relaxed text-ink4">
                      {item.desc}
                    </p>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {/* Language Selection Card */}
        <div className="rounded-card border border-line bg-card p-5 space-y-3">
          <div className="border-b border-line pb-3">
            <h3 className="t-section-title">
              <Globe className="h-4 w-4 text-accent" />
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
                  className={`flex items-center justify-between p-3 rounded-panel border text-xs font-medium transition-all cursor-pointer ${
                    isSelected
                      ? "border-linestrong bg-hovert text-ink shadow-sm"
                      : "border-line bg-cardin text-ink4 hover:text-ink2 hover:bg-raise"
                  }`}
                >
                  <span className="font-semibold">{item.full}</span>
                  <span className="font-mono text-[10px] text-ink5">{item.label}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Security & Warning Against Fakes */}
        <div className="rounded-card border border-warnline bg-warnwash p-4 space-y-2">
          <div className="flex items-center gap-2 text-amber-400">
            <AlertTriangle className="h-4 w-4" />
            <h4 className="text-xs font-semibold text-ink">{t.settingsTab.warningFakesTitle}</h4>
          </div>
          <p className="text-[11px] text-ink3 leading-relaxed">
            {t.settingsTab.warningFakesBody}
          </p>
          <div className="flex flex-wrap gap-4 pt-1 text-xs">
            {EXTERNAL_LINKS.map((l) => (
              <button
                key={l.url}
                type="button"
                onClick={() => openUrl(l.url)}
                className="flex items-center gap-1.5 text-accent hover:text-accent-soft hover:underline cursor-pointer bg-transparent border-0 p-0"
              >
                <span>{l.label}</span>
                <ExternalLink className="h-3 w-3" />
              </button>
            ))}
          </div>
        </div>

        {/* Application Architecture & Versions */}
        <div className="rounded-card border border-line bg-card p-4">
          <div className="flex items-center justify-between text-xs text-ink4">
            <div className="flex items-center gap-2">
              <ZapretLogo className="h-4 w-4" />
              <span className="font-semibold text-ink">zapret2</span>
              <span className="font-mono text-accent font-semibold">{t.header.version}</span>
            </div>
            <div className="font-mono text-[11px] text-ink5">
              WinWS v2.1.8 • WinDivert v2.2.2-F • Tauri v2
            </div>
          </div>
        </div>
      </div>
    );
  }
);
