import React, { useState } from "react";
import { DiagnosticsCard } from "../DiagnosticsCard";
import { NetworkDoctorModal } from "../NetworkDoctorModal";
import { DiagnosticsReport } from "../../types";
import { Wrench, Activity, ShieldCheck, Zap } from "lucide-react";
import { useI18n } from "../../i18n";

interface DiagnosticsViewProps {
  diagnostics: DiagnosticsReport;
  onRefreshProbes: () => void;
  isRefreshingProbes: boolean;
  onSelectAndStartStrategy: (id: string) => void;
}

export const DiagnosticsView: React.FC<DiagnosticsViewProps> = React.memo(
  ({ diagnostics, onRefreshProbes, isRefreshingProbes, onSelectAndStartStrategy }) => {
    const { t } = useI18n();
    const [isDoctorOpen, setIsDoctorOpen] = useState(false);

    return (
      <div className="space-y-6 pb-6">
        {/* Banner with Doctor Quick Action */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 rounded-panel border border-warnline bg-warnwash p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-amber-500/10 border border-amber-500/25 text-amber-400">
              <Wrench className="h-4 w-4" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-ink flex items-center gap-2">
                <span>{t.doctor.title}</span>
                <span className="rounded-chip bg-amber-500/15 border border-amber-500/25 px-1.5 py-0.5 text-[9px] font-medium text-amber-300">
                  Self-Healing
                </span>
              </h3>
              <p className="text-[11px] text-ink4 mt-0.5">
                Автоматическое исправление сбоев сети: сброс кэша DNS, службы WinDivert и зависших сокетов
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setIsDoctorOpen(true)}
            className="flex items-center justify-center gap-2 rounded-control bg-amber-500 hover:bg-amber-400 text-slate-950 font-semibold px-4 py-2 text-xs transition-all active:scale-[0.98] cursor-pointer shrink-0"
          >
            <Wrench className="h-3.5 w-3.5" />
            <span>Запустить Авто-Доктор</span>
          </button>
        </div>

        {/* Real-Time Probes & Auto-Tune Card */}
        <DiagnosticsCard
          report={diagnostics}
          onRefresh={onRefreshProbes}
          isRefreshing={isRefreshingProbes}
          onSelectAndStartStrategy={onSelectAndStartStrategy}
        />

        {/* Network Doctor Modal */}
        <NetworkDoctorModal
          isOpen={isDoctorOpen}
          onClose={() => setIsDoctorOpen(false)}
          onSuccessRefresh={onRefreshProbes}
        />
      </div>
    );
  }
);
