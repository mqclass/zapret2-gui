import React from "react";
import {
  ShieldCheck,
  ShieldAlert,
  Rocket,
  Trash2,
  Globe,
  RefreshCw,
  Sparkles,
} from "lucide-react";
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

    const getTabMeta = (tab: NavTab) => {
      switch (tab) {
        case "dashboard":
          return {
            title: "Центр управления обходом",
            subtitle: "Основной шлюз фильтрации пакетов WFP / WinDivert",
          };
        case "strategies":
          return {
            title: "Библиотека пресетов и стратегий",
            subtitle: "Выбор оптимального алгоритма desync для YouTube, Discord и сайтов",
          };
        case "diagnostics":
          return {
            title: "Диагностика сети & Авто-Доктор",
            subtitle: "Мониторинг пинга сервисов, автоподбор стратегии и лечение сети",
          };
        case "dns":
          return {
            title: "DNS & Тест скорости YouTube 4K",
            subtitle: "Защита от подмены DNS провайдером и замер реального видеопотока",
          };
        case "lists":
          return {
            title: "Маршрутизация и Списки доменов",
            subtitle: "Синхронизация списков с GitHub и управление своими хостами",
          };
        case "logs":
          return {
            title: "Системный журнал событий",
            subtitle: "Консольный вывод процессов winws2 и службы супервизора",
          };
        case "settings":
          return {
            title: "Параметры и Информация",
            subtitle: "Конфигурация автозапуска, очистка, лицензии и безопасность",
          };
      }
    };

    const meta = getTabMeta(activeTab);

    const languages: { code: Language; label: string }[] = [
      { code: "ru", label: "RU" },
      { code: "en", label: "EN" },
      { code: "uk", label: "UK" },
    ];

    return (
      <header className="h-16 shrink-0 flex items-center justify-between px-6 border-b border-white/[0.06] bg-slate-950/70 backdrop-blur-xl select-none">
        {/* Title area */}
        <div>
          <h2 className="text-sm font-bold tracking-tight text-white">{meta.title}</h2>
          <p className="text-[11px] text-slate-400 font-medium">{meta.subtitle}</p>
        </div>

        {/* Right side quick actions */}
        <div className="flex items-center gap-2.5">
          {/* UAC Elevation Status */}
          {isElevated ? (
            <div
              className="flex items-center gap-1.5 rounded-lg border border-emerald-500/25 bg-emerald-500/10 px-2.5 py-1 text-[11px] font-semibold text-emerald-300"
              title="Приложение запущено с повышенными правами Администратора (WFP доступен)"
            >
              <ShieldCheck className="h-3.5 w-3.5 text-emerald-400" />
              <span>Админ</span>
            </div>
          ) : (
            <button
              onClick={onRelaunchElevated}
              className="flex items-center gap-1.5 rounded-lg border border-amber-500/40 bg-amber-500/15 px-2.5 py-1 text-[11px] font-semibold text-amber-300 hover:bg-amber-500/25 transition-all cursor-pointer animate-pulse"
              title="Нажмите для перезапуска с правами Администратора (требуется для WFP)"
            >
              <ShieldAlert className="h-3.5 w-3.5 text-amber-400" />
              <span>Нужен Админ</span>
            </button>
          )}

          {/* Quick Autostart Toggle */}
          <button
            onClick={onToggleAutostart}
            disabled={isTogglingAutostart}
            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-[11px] font-medium transition-all cursor-pointer disabled:opacity-60 ${
              isAutostart
                ? "bg-indigo-600/15 border-indigo-500/35 text-indigo-300 hover:bg-indigo-600/25"
                : "bg-white/[0.03] border-white/[0.08] text-slate-400 hover:text-slate-200 hover:bg-white/[0.06]"
            }`}
            title={isAutostart ? "Автозапуск включен" : "Автозапуск выключен"}
          >
            {isTogglingAutostart ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-indigo-400" />
            ) : (
              <Rocket
                className={`h-3.5 w-3.5 ${isAutostart ? "text-indigo-400" : "text-slate-500"}`}
              />
            )}
            <span>Автозапуск</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                isAutostart ? "bg-indigo-400 shadow-[0_0_6px_#818cf8]" : "bg-slate-600"
              }`}
            />
          </button>

          {/* Quick Stray Process Cleaner */}
          <button
            onClick={onKillStray}
            disabled={isStrayCleaning}
            className="flex items-center gap-1.5 rounded-lg border border-white/[0.08] bg-white/[0.03] px-2.5 py-1 text-[11px] font-medium text-slate-400 hover:text-rose-300 hover:border-rose-500/30 hover:bg-rose-500/10 transition-all cursor-pointer disabled:opacity-60"
            title="Завершить потерянные процессы winws2 в памяти"
          >
            {isStrayCleaning ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-rose-400" />
            ) : (
              <Trash2 className="h-3.5 w-3.5" />
            )}
            <span className="hidden sm:inline">Очистить</span>
          </button>

          {/* Language Switcher */}
          <div className="flex items-center gap-0.5 rounded-lg border border-white/[0.08] bg-slate-900/60 p-0.5">
            <Globe className="h-3 w-3 ml-1 text-slate-500 mr-0.5" />
            {languages.map((l) => (
              <button
                key={l.code}
                onClick={() => setLanguage(l.code)}
                className={`rounded px-1.5 py-0.5 text-[10px] font-bold transition-all cursor-pointer ${
                  language === l.code
                    ? "bg-blue-600 text-white shadow-xs"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {l.label}
              </button>
            ))}
          </div>
        </div>
      </header>
    );
  }
);
