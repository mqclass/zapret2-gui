import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Wrench,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Loader2,
  X,
  Activity,
  ShieldCheck,
  RefreshCw,
} from "lucide-react";
import { DoctorReport } from "../types";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";

interface NetworkDoctorModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccessRefresh?: () => void;
}

export const NetworkDoctorModal: React.FC<NetworkDoctorModalProps> = ({
  isOpen,
  onClose,
  onSuccessRefresh,
}) => {
  const { t } = useI18n();
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [report, setReport] = useState<DoctorReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen) return null;

  const handleRunDoctor = async () => {
    setIsRunning(true);
    setError(null);
    setReport(null);

    try {
      const result = await invoke<DoctorReport>("run_auto_doctor");
      setReport(result);
      if (onSuccessRefresh) {
        onSuccessRefresh();
      }
    } catch (err: any) {
      console.error("Auto-Doctor execution failed:", err);
      setError(formatErrorMessage(err));
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-overlay backdrop-blur-md p-4">
      <div className="relative flex max-h-[90vh] w-full max-w-2xl flex-col rounded-card border border-raise bg-card shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between border-b border-raise p-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-panel bg-gradient-to-tr from-amber-500 to-rose-500 shadow-md shadow-amber-500/20">
              <Wrench className="h-5 w-5 text-ink" />
            </div>
            <div>
              <h2 className="text-base font-bold text-ink">{t.doctor.title}</h2>
              <p className="text-xs text-ink4 mt-0.5">{t.doctor.subtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-control p-1.5 text-ink4 hover:bg-raise hover:text-ink transition-colors cursor-pointer"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto p-5 space-y-4">
          {error && (
            <div className="flex items-start gap-3 rounded-panel border border-rose-500/30 bg-rose-500/10 p-4 text-xs text-rose-300">
              <AlertTriangle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
              <div>
                <span className="font-semibold text-rose-200">{t.doctor.errorTitle}</span>
                <p className="mt-0.5 leading-relaxed">{error}</p>
              </div>
            </div>
          )}

          {/* Intro Card */}
          <div className="rounded-panel border border-raise/80 bg-cardin/60 p-4 text-xs text-ink3 leading-relaxed space-y-2">
            <div className="flex items-center gap-2 font-semibold text-ink">
              <Activity className="h-4 w-4 text-amber-400" />
              <span>{t.doctor.introTitle}</span>
            </div>
            <ul className="list-disc list-inside space-y-1 text-ink4 pl-1">
              <li>{t.doctor.introItem1a} <code className="text-amber-300">winws2.exe</code> {t.doctor.introItem1b} <code className="text-amber-300">winws.exe</code></li>
              <li>{t.doctor.introItem2} <code className="text-accent-soft">WinDivert</code></li>
              <li>{t.doctor.introItem3} (<code className="text-indigo-300">ipconfig /flushdns</code>)</li>
              <li>{t.doctor.introItem4}</li>
              <li>{t.doctor.introItem5} (<code className="text-emerald-300">netsh int tcp set global timestamps=enabled</code>)</li>
              <li>{t.doctor.introItem6}</li>
            </ul>
          </div>

          {/* Action Button */}
          <div className="flex justify-center pt-2">
            <button
              onClick={handleRunDoctor}
              disabled={isRunning}
              className="flex items-center gap-2.5 rounded-panel bg-gradient-to-r from-amber-500 to-rose-600 px-6 py-3 font-semibold text-sm text-ink shadow-lg shadow-amber-500/20 hover:from-amber-400 hover:to-rose-500 active:scale-95 transition-all cursor-pointer disabled:opacity-50"
            >
              {isRunning ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>{t.doctor.running}</span>
                </>
              ) : (
                <>
                  <Wrench className="h-4 w-4" />
                  <span>{t.doctor.runBtn}</span>
                </>
              )}
            </button>
          </div>

          {/* Report Results */}
          {report && (
            <div className="mt-4 space-y-3">
              <div
                className={`flex items-center gap-2.5 rounded-panel border p-3 text-xs font-semibold ${
                  report.success
                    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                    : "border-amber-500/30 bg-amber-500/10 text-amber-300"
                }`}
              >
                {report.success ? (
                  <ShieldCheck className="h-4 w-4 text-emerald-400" />
                ) : (
                  <AlertTriangle className="h-4 w-4 text-amber-400" />
                )}
                <span>{report.message || report.summary}</span>
              </div>

              <div className="divide-y divide-line/80 rounded-panel border border-raise bg-cardin/60 overflow-hidden">
                {report.steps.map((step) => {
                  let statusBadge = (
                    <span className="flex items-center gap-1 text-[11px] font-semibold text-emerald-400 bg-emerald-500/10 border border-emerald-500/20 px-2 py-0.5 rounded-chip">
                      <CheckCircle2 className="h-3 w-3" />
                      {t.doctor.stepOk}
                    </span>
                  );

                  if (step.status === "fixed") {
                    statusBadge = (
                      <span className="flex items-center gap-1 text-[11px] font-semibold text-accent bg-accent-wash border border-accent-line px-2 py-0.5 rounded-chip">
                        <RefreshCw className="h-3 w-3" />
                        {t.doctor.stepFixed}
                      </span>
                    );
                  } else if (step.status === "warn") {
                    statusBadge = (
                      <span className="flex items-center gap-1 text-[11px] font-semibold text-amber-400 bg-amber-500/10 border border-amber-500/20 px-2 py-0.5 rounded-chip">
                        <AlertTriangle className="h-3 w-3" />
                        {t.doctor.warnBadge}
                      </span>
                    );
                  } else if (step.status === "failed" || step.status === "error") {
                    statusBadge = (
                      <span className="flex items-center gap-1 text-[11px] font-semibold text-rose-400 bg-rose-500/10 border border-rose-500/20 px-2 py-0.5 rounded-chip">
                        <XCircle className="h-3 w-3" />
                        {t.doctor.stepFailed}
                      </span>
                    );
                  }

                  return (
                    <div key={step.id} className="p-3.5 flex items-start justify-between gap-3">
                      <div>
                        <div className="font-semibold text-xs text-ink2">
                          {step.title || step.name}
                        </div>
                        <div className="text-[11px] text-ink4 mt-0.5">
                          {step.details || step.message}
                        </div>
                      </div>
                      <div className="shrink-0">{statusBadge}</div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end border-t border-raise p-4">
          <button
            onClick={onClose}
            className="rounded-panel border border-linestrong bg-raise px-4 py-2 text-xs font-semibold text-ink2 hover:bg-linestrong transition-colors cursor-pointer"
          >
            {t.doctor.closeBtn}
          </button>
        </div>
      </div>
    </div>
  );
};
