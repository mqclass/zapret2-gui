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
  AppWindow,
  ListPlus,
} from "lucide-react";
import { ZapretLogo } from "../ZapretLogo";
import { useI18n } from "../../i18n";
import { Language } from "../../i18n/types";
import { openUrl } from "@tauri-apps/plugin-opener";
import { UpdateInfo } from "../../types";
import { AppSplitModal } from "../AppSplitModal";
import {
  ACCENT_PRESETS,
  applyAccent,
  clearDraftAccent,
  clearDraftAccentIf,
  currentAccentFill,
  writeDraftAccent,
} from "../../utils/accent";

/** External links opened via the opener plugin (L-2). The capability allowlist
 *  restricts this to https://github.com/* — no webview navigation is enabled. */
const EXTERNAL_LINKS: { url: string; label: string; labelKey?: "flowsealLink" }[] = [
  { url: "https://github.com/bol-van/zapret", label: "GitHub bol-van/zapret" },
  { url: "https://github.com/Flowseal/zapret-discord-youtube", label: "GitHub Flowseal", labelKey: "flowsealLink" },
  { url: "https://github.com/mqclass/zapret2-gui", label: "GitHub mqclass/zapret2-gui" },
];


interface SettingsViewProps {
  /** Shell view mode (redesign): false = Простой вид, true = Эксперт. */
  expertMode?: boolean;
  /** Switch the shell to expert mode (simple view's "show all settings"). */
  onExpertModeChange?: (expert: boolean) => void;
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
  watchdogOn: boolean;
  onToggleWatchdog: () => void;
  isTogglingWatchdog: boolean;
  autoHostlistOn: boolean;
  onToggleAutoHostlist: () => void;
  isTogglingAutoHostlist: boolean;
  watchdogInterval: number;
  onWatchdogInterval: (minutes: number) => void;
  accentColor: string | null;
  onAccentChange: (color: string | null) => Promise<void> | void;
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
    expertMode = false,
    onExpertModeChange,
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
    watchdogOn,
    onToggleWatchdog,
    isTogglingWatchdog,
    autoHostlistOn,
    onToggleAutoHostlist,
    isTogglingAutoHostlist,
    watchdogInterval,
    onWatchdogInterval,
    accentColor,
    onAccentChange,
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
    const [isAppSplitOpen, setIsAppSplitOpen] = React.useState(false);

    // Custom color picker: preview instantly, save debounced. The native
    // picker fires onChange on every drag step, and each save rewrites
    // the whole settings file, so drag events must not reach the backend
    // directly. The draft stays local until flushed (450ms debounce,
    // blur, or unmount); a failed save drops the draft so the picker
    // falls back to the real saved color instead of showing an unsaved one.
    const [customDraft, setCustomDraft] = React.useState<string | null>(null);
    const pendingAccentSaveRef = React.useRef<{
      value: string;
      timer: number;
    } | null>(null);
    const accentColorRef = React.useRef(accentColor);
    accentColorRef.current = accentColor;
    const onAccentChangeRef = React.useRef(onAccentChange);
    onAccentChangeRef.current = onAccentChange;
    const flushCustomAccentSave = React.useCallback(() => {
      const pending = pendingAccentSaveRef.current;
      if (!pending) return;
      pendingAccentSaveRef.current = null;
      window.clearTimeout(pending.timer);
      const current = currentAccentFill(accentColorRef.current);
      if (pending.value.toLowerCase() === current.toLowerCase()) {
        setCustomDraft(null);
        clearDraftAccentIf(pending.value);
        return;
      }
      const result = onAccentChangeRef.current(pending.value);
      if (result && typeof result.then === "function") {
        result.then(
          () => {
            setCustomDraft(null);
            clearDraftAccentIf(pending.value);
          },
          () => {
            // Save failed: backend value unchanged, revert the preview.
            setCustomDraft(null);
            clearDraftAccentIf(pending.value);
            applyAccent(accentColorRef.current);
          }
        );
      } else {
        setCustomDraft(null);
        clearDraftAccentIf(pending.value);
      }
    }, []);
    const flushCustomAccentSaveRef = React.useRef(flushCustomAccentSave);
    flushCustomAccentSaveRef.current = flushCustomAccentSave;
    // A pending color must not be lost when leaving the tab mid-debounce.
    React.useEffect(
      () => () => {
        flushCustomAccentSaveRef.current();
      },
      []
    );
    const cancelCustomAccentSave = React.useCallback(() => {
      const pending = pendingAccentSaveRef.current;
      if (pending) {
        window.clearTimeout(pending.timer);
        pendingAccentSaveRef.current = null;
      }
      setCustomDraft(null);
      clearDraftAccent();
    }, []);
    // Shared by the simple and expert branches: the preset click and the
    // custom-color drag must behave identically in both views, so the
    // handlers live here instead of being duplicated per branch.
    const handlePresetClick = React.useCallback(
      (preset: (typeof ACCENT_PRESETS)[number]) => {
        cancelCustomAccentSave();
        const res = onAccentChangeRef.current(
          preset.id === "violet" ? null : preset.fill
        );
        // A failed preset save is already surfaced by App
        // (error alert); swallowing here keeps the fire-and-
        // forget click from becoming an unhandled rejection.
        if (res && typeof res.then === "function") {
          res.catch(() => {});
        }
      },
      [cancelCustomAccentSave]
    );
    const handleCustomColorChange = React.useCallback(
      (v: string) => {
        setCustomDraft(v);
        applyAccent(v);
        // Survives closing the app mid-debounce: App reconciles
        // this draft once on the next launch (see accent.ts).
        writeDraftAccent(v);
        const pending = pendingAccentSaveRef.current;
        if (pending) window.clearTimeout(pending.timer);
        pendingAccentSaveRef.current = {
          value: v,
          timer: window.setTimeout(() => flushCustomAccentSave(), 450),
        };
      },
      [flushCustomAccentSave]
    );
    const presetLabel = (preset: (typeof ACCENT_PRESETS)[number]) =>
      preset.id === "violet"
        ? t.settingsTab.accentViolet
        : preset.id === "cyan"
          ? t.settingsTab.accentCyan
          : preset.id === "emerald"
            ? t.settingsTab.accentEmerald
            : preset.id === "amber"
              ? t.settingsTab.accentAmber
              : preset.id === "pink"
                ? t.settingsTab.accentPink
                : t.settingsTab.accentBlue;
    const languages: { code: Language; label: string; full: string }[] = [
      { code: "ru", label: "RU", full: "Русский" },
      { code: "en", label: "EN", full: "English" },
      { code: "uk", label: "UK", full: "Українська" },
    ];

    // ------------------------------------------------------------------
    // Простой вид (redesign stage 6): цвет, язык, сторож, обновления.
    // Fits 1050x760 without scrolling. Everything else (автозапуск,
    // служба, уведомления, экспорт диагностики, сплит приложений)
    // lives in the expert branch below. All handlers are the same
    // ones the full view uses; the update dialog and banner stay
    // global (the banner sits above every tab in App).
    // ------------------------------------------------------------------
    if (!expertMode) {
      const currentFill = currentAccentFill(accentColor);
      // Same rule as the full view's select: a backend interval
      // outside 5/15/30/60 still gets its own chip.
      const intervalOptions = [...new Set([5, 15, 30, 60, watchdogInterval])].sort(
        (a, b) => a - b
      );
      // Simple-view cards use p-3 (not the p-4 of the expert cards):
      // at p-4 the "show all settings" link sat a few pixels from the
      // bottom edge at 1050x760, and another locale or font scale
      // would push it into the scroll. The tighter padding buys ~30px
      // of slack below the link. No bottom padding of our own: the
      // shell's main already has py-5, and even pb-2 left the view
      // 7px scrollable at 1050x760.
      return (
        <div className="space-y-3 max-w-4xl">
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
            {/* Accent color — full width */}
            <div className="t-card p-3 space-y-2 lg:col-span-2 stagger-item" style={{ "--stagger-i": 0 } as React.CSSProperties}>
              <div className="flex items-center gap-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-accent-fill/20 to-accent-fill/5 border border-accent-line text-accent">
                  <Palette className="h-4 w-4" />
                </div>
                <h3 className="text-xs font-semibold text-ink">{t.settingsTab.accentTitle}</h3>
              </div>

              <div className="grid grid-cols-3 sm:grid-cols-6 gap-2">
                {ACCENT_PRESETS.map((preset) => {
                  const isSelected =
                    currentFill.toLowerCase() === preset.fill.toLowerCase();
                  const label = presetLabel(preset);
                  return (
                    <button
                      type="button"
                      key={preset.id}
                      onClick={() => handlePresetClick(preset)}
                      title={label}
                      className={`flex flex-col items-center gap-1.5 p-2 rounded-panel border text-xs font-medium transition-all cursor-pointer ${
                        isSelected
                          ? "border-linestrong bg-hovert text-ink shadow-sm"
                          : "border-line bg-cardin text-ink4 hover:text-ink2 hover:bg-raise"
                      }`}
                    >
                      <span
                        className="flex h-6 w-6 items-center justify-center rounded-full border border-white/20"
                        style={{ backgroundColor: preset.fill }}
                      >
                        {isSelected && <Check className="h-3.5 w-3.5 text-white drop-shadow" />}
                      </span>
                      <span>{label}</span>
                    </button>
                  );
                })}
              </div>

              <label className="flex items-center gap-2.5 rounded-panel border border-line bg-cardin px-3 py-1.5 text-xs text-ink4 cursor-pointer">
                <input
                  type="color"
                  value={customDraft ?? currentFill}
                  onChange={(e) => handleCustomColorChange(e.target.value)}
                  onBlur={() => flushCustomAccentSave()}
                  className="h-6 w-9 shrink-0 cursor-pointer rounded-control border border-line bg-transparent p-0.5"
                />
                <span className="font-medium">{t.settingsTab.accentCustom}</span>
                <span className="font-mono text-[11px] text-ink5">
                  {customDraft ?? currentFill}
                </span>
              </label>
            </div>

            {/* Language. The card stretches to the watchdog card's
                height, so the buttons sit centered in the leftover
                space. No language code here: "Українська" + "UK" did
                not fit a third of a half-width card and truncated. */}
            <div className="t-card p-3 flex flex-col stagger-item" style={{ "--stagger-i": 1 } as React.CSSProperties}>
              <div className="flex items-center gap-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-accent-fill/20 to-accent-fill/5 border border-accent-line text-accent">
                  <Globe className="h-4 w-4" />
                </div>
                <h3 className="text-xs font-semibold text-ink">{t.settingsTab.languageSection}</h3>
              </div>

              <div className="flex-1 flex items-center">
                <div className="grid grid-cols-3 gap-2 w-full">
                  {languages.map((item) => {
                    const isSelected = language === item.code;
                    return (
                      <button
                        type="button"
                        key={item.code}
                        onClick={() => setLanguage(item.code)}
                        className={`flex items-center justify-center p-2.5 rounded-panel border text-xs font-medium transition-all cursor-pointer ${
                          isSelected
                            ? "border-linestrong bg-hovert text-ink shadow-sm"
                            : "border-line bg-cardin text-ink4 hover:text-ink2 hover:bg-raise"
                        }`}
                      >
                        <span className="font-semibold">{item.full}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>

            {/* Watchdog */}
            <div className="t-card p-3 space-y-2 stagger-item" style={{ "--stagger-i": 1 } as React.CSSProperties}>
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-3 min-w-0">
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-accent-fill/20 to-accent-fill/5 border border-accent-line text-accent">
                    <ShieldCheck className="h-4 w-4" />
                  </div>
                  <h3 className="text-xs font-semibold text-ink truncate">{t.settingsTab.watchdogTitle}</h3>
                </div>
                <span
                  className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium shrink-0 ${
                    watchdogOn
                      ? "bg-accent-wash text-accent-soft border border-accent-line"
                      : "bg-hovert text-ink4"
                  }`}
                >
                  {watchdogOn ? t.settingsTab.watchdogOn : t.settingsTab.watchdogOff}
                </span>
              </div>

              <p className="text-[11px] text-ink4 leading-relaxed">
                {t.settingsTab.watchdogShort}
              </p>

              {/* Label above the chips: label + 4 chips on one line
                  wrapped "60 мин" onto a second row. */}
              <div className="text-[11px] text-ink4">{t.settingsTab.watchdogIntervalLabel}</div>
              <div className="flex items-center gap-2 flex-wrap">
                {intervalOptions.map((n) => {
                  const isSelected = watchdogInterval === n;
                  return (
                    <button
                      type="button"
                      key={n}
                      onClick={() => onWatchdogInterval(n)}
                      className={`rounded-control px-2.5 py-1 text-xs font-semibold transition-all cursor-pointer border ${
                        isSelected
                          ? "bg-accent-wash text-accent-soft border-accent-line"
                          : "bg-cardin text-ink3 border-line hover:bg-raise"
                      }`}
                    >
                      {n} {t.settingsTab.watchdogMinutes}
                    </button>
                  );
                })}
              </div>

              <button
                type="button"
                onClick={onToggleWatchdog}
                disabled={isTogglingWatchdog}
                className="w-full flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isTogglingWatchdog ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : (
                  <ShieldCheck className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {watchdogOn
                    ? t.settingsTab.watchdogBtnDisable
                    : t.settingsTab.watchdogBtnEnable}
                </span>
              </button>
            </div>

            {/* Updates — full width */}
            <div className="t-card p-3 space-y-2 lg:col-span-2 stagger-item" style={{ "--stagger-i": 2 } as React.CSSProperties}>
              <div className="flex items-center gap-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-accent-fill/20 to-accent-fill/5 border border-accent-line text-accent">
                  <ArrowDownToLine className="h-4 w-4" />
                </div>
                <h3 className="text-xs font-semibold text-ink">{t.update.sectionTitle}</h3>
              </div>

              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-2.5">
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    {updateInfo?.update_available ? (
                      <AlertTriangle className="h-4 w-4 text-amber-400" />
                    ) : (
                      <BadgeCheck className="h-4 w-4 text-accent" />
                    )}
                    <span className="text-xs font-semibold text-ink">{t.update.versionTitle}</span>
                    <span className="rounded-chip px-1.5 py-0.5 text-[10px] font-mono bg-hovert text-ink4">
                      v{updateInfo?.current_version ?? t.header.version.replace(/^v/, "")}
                    </span>
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

              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-2.5">
                <div className="flex items-center gap-2">
                  <RefreshCw
                    className={`h-4 w-4 ${autoUpdateOn ? "text-accent" : "text-ink4"}`}
                  />
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
                    {autoUpdateOn ? t.update.autoCheckDisable : t.update.autoCheckEnable}
                  </span>
                </button>
              </div>
            </div>
          </div>

          {/* Bridge to the expert view (same text-button pattern as the
              log digest's "show full log"). */}
          <button
            type="button"
            onClick={() => onExpertModeChange?.(true)}
            className="text-xs text-accent-soft hover:text-ink transition-colors cursor-pointer"
          >
            {t.settingsTab.showAllSettings} →
          </button>
        </div>
      );
    }

    return (
      <div className="space-y-4 pb-4 max-w-4xl">
        {/* System & Engine Section */}
        <div className="t-card p-5 space-y-4 stagger-item" style={{ "--stagger-i": 0 } as React.CSSProperties}>
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
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.autostartTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      isAutostart
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {isAutostart ? t.settingsTab.autostartOn : t.settingsTab.autostartOff}
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
                <span>{isAutostart ? t.settingsTab.autostartDisable : t.settingsTab.autostartEnable}</span>
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
                    {isElevated ? t.settingsTab.elevatedYes : t.settingsTab.elevatedNo}
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

            {/* Strategy Watchdog Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <ShieldCheck className="h-4 w-4 text-accent" />
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.watchdogTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      watchdogOn
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {watchdogOn ? t.settingsTab.watchdogOn : t.settingsTab.watchdogOff}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.watchdogDesc}
                </p>
                <label className="flex items-center gap-2 pt-1 text-[11px] text-ink4">
                  <span>{t.settingsTab.watchdogIntervalLabel}</span>
                  <select
                    value={watchdogInterval}
                    onChange={(e) => onWatchdogInterval(Number(e.target.value))}
                    className="rounded-control border border-line bg-cardin px-2 py-1 text-xs text-ink2 focus:border-accent-line focus:outline-none cursor-pointer"
                  >
                    {[...new Set([5, 15, 30, 60, watchdogInterval])]
                      .sort((a, b) => a - b)
                      .map((n) => (
                        <option key={n} value={n}>
                          {n} {t.settingsTab.watchdogMinutes}
                        </option>
                      ))}
                  </select>
                </label>
              </div>

              <button
                type="button"
                onClick={onToggleWatchdog}
                disabled={isTogglingWatchdog}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isTogglingWatchdog ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : (
                  <ShieldCheck className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {watchdogOn
                    ? t.settingsTab.watchdogBtnDisable
                    : t.settingsTab.watchdogBtnEnable}
                </span>
              </button>
            </div>

            {/* Auto-Hostlist Card */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-panel border border-line bg-cardin p-3.5">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <ListPlus className="h-4 w-4 text-accent" />
                  <span className="text-xs font-semibold text-ink">{t.settingsTab.autoHostlistTitle}</span>
                  <span
                    className={`rounded-chip px-1.5 py-0.5 text-[10px] font-medium ${
                      autoHostlistOn
                        ? "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert text-ink4"
                    }`}
                  >
                    {autoHostlistOn ? t.settingsTab.autoHostlistOn : t.settingsTab.autoHostlistOff}
                  </span>
                </div>
                <p className="text-[11px] text-ink4 max-w-lg leading-relaxed">
                  {t.settingsTab.autoHostlistDesc}
                </p>
              </div>

              <button
                type="button"
                onClick={onToggleAutoHostlist}
                disabled={isTogglingAutoHostlist}
                className="flex items-center justify-center gap-2 rounded-control px-3.5 py-2 text-xs font-semibold transition-all cursor-pointer disabled:opacity-50 shrink-0 border border-line bg-hovert hover:bg-raise text-ink2"
              >
                {isTogglingAutoHostlist ? (
                  <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                ) : (
                  <ListPlus className="h-3.5 w-3.5 text-accent" />
                )}
                <span>
                  {autoHostlistOn
                    ? t.settingsTab.autoHostlistBtnDisable
                    : t.settingsTab.autoHostlistBtnEnable}
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
                  {autoUpdateOn ? t.update.autoCheckDisable : t.update.autoCheckEnable}
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

        {/* Language Selection Card */}
        <div className="t-card p-5 space-y-3 stagger-item" style={{ "--stagger-i": 2 } as React.CSSProperties}>
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

        {/* Accent Color Card */}
        <div className="t-card p-5 space-y-3 stagger-item" style={{ "--stagger-i": 2 } as React.CSSProperties}>
          <div className="border-b border-line pb-3">
            <h3 className="t-section-title">
              <Palette className="h-4 w-4 text-accent" />
              <span>{t.settingsTab.accentTitle}</span>
            </h3>
          </div>
          <p className="text-[11px] text-ink4 leading-relaxed">{t.settingsTab.accentDesc}</p>

          <div className="grid grid-cols-3 sm:grid-cols-6 gap-2.5">
            {ACCENT_PRESETS.map((preset) => {
              const currentFill = currentAccentFill(accentColor);
              const isSelected =
                currentFill.toLowerCase() === preset.fill.toLowerCase();
              const label = presetLabel(preset);
              return (
                <button
                  type="button"
                  key={preset.id}
                  onClick={() => handlePresetClick(preset)}
                  title={label}
                  className={`flex flex-col items-center gap-1.5 p-3 rounded-panel border text-xs font-medium transition-all cursor-pointer ${
                    isSelected
                      ? "border-linestrong bg-hovert text-ink shadow-sm"
                      : "border-line bg-cardin text-ink4 hover:text-ink2 hover:bg-raise"
                  }`}
                >
                  <span
                    className="flex h-7 w-7 items-center justify-center rounded-full border border-white/20"
                    style={{ backgroundColor: preset.fill }}
                  >
                    {isSelected && <Check className="h-4 w-4 text-white drop-shadow" />}
                  </span>
                  <span>{label}</span>
                </button>
              );
            })}
          </div>

          <label className="flex items-center gap-2.5 rounded-panel border border-line bg-cardin px-3 py-2.5 text-xs text-ink4 cursor-pointer">
            <input
              type="color"
              value={customDraft ?? currentAccentFill(accentColor)}
              onChange={(e) => handleCustomColorChange(e.target.value)}
              onBlur={() => flushCustomAccentSave()}
              className="h-7 w-10 shrink-0 cursor-pointer rounded-control border border-line bg-transparent p-0.5"
            />
            <span className="font-medium">{t.settingsTab.accentCustom}</span>
            <span className="font-mono text-[11px] text-ink5">
              {customDraft ?? currentAccentFill(accentColor)}
            </span>
          </label>
        </div>

        {/* Per-app split tunneling */}
        <div className="t-card p-5 space-y-3 stagger-item" style={{ "--stagger-i": 3 } as React.CSSProperties}>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-panel bg-accent-wash border border-accent-line text-accent">
                <AppWindow className="h-4 w-4" />
              </div>
              <div>
                <h3 className="text-xs font-semibold text-ink">{t.appSplit.title}</h3>
                <p className="text-[11px] text-ink4">{t.appSplit.subtitle}</p>
              </div>
            </div>
            <button
              onClick={() => setIsAppSplitOpen(true)}
              className="rounded-control px-3 py-1.5 text-xs font-semibold bg-hovert hover:bg-accent-fill hover:text-accent-contrast text-ink3 border border-line transition-all cursor-pointer"
            >
              {t.appSplit.configure}
            </button>
          </div>
        </div>
        <AppSplitModal isOpen={isAppSplitOpen} onClose={() => setIsAppSplitOpen(false)} />

        {/* Security & Warning Against Fakes */}
        <div className="rounded-card border border-warnline bg-warnwash p-4 space-y-2 stagger-item" style={{ "--stagger-i": 3 } as React.CSSProperties}>
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
                <span>{l.labelKey ? t.settingsTab[l.labelKey] : l.label}</span>
                <ExternalLink className="h-3 w-3" />
              </button>
            ))}
          </div>
        </div>

        {/* Application Architecture & Versions */}
        <div className="t-card p-4 stagger-item" style={{ "--stagger-i": 4 } as React.CSSProperties}>
          <div className="flex items-center justify-between text-xs text-ink4">
            <div className="flex items-center gap-2">
              <ZapretLogo className="h-4 w-4" />
              <span className="font-semibold text-ink">zapret2</span>
              <span className="font-mono text-accent font-semibold">{t.header.version}</span>
            </div>
            <div className="font-mono text-[11px] text-ink5">
              {/* Each "name + version" pair is unbreakable, and the "•"
                  separator is glued to the pair on its LEFT: a wrap can
                  then only happen after a "•", never before one, so the
                  separator can never land at the start of a line and a
                  name can never split from its version. */}
              <span className="whitespace-nowrap">WinWS v2.1.8 •</span>{" "}
              <span className="whitespace-nowrap">WinDivert v2.2.2-F •</span>{" "}
              <span className="whitespace-nowrap">Tauri v2</span>
            </div>
            <div className="font-mono text-[10px] text-ink5">
              Telegram bridge: tg-ws-proxy-rs v2.5.4 (MIT, © valnesfjord)
            </div>
          </div>
        </div>
      </div>
    );
  }
);
