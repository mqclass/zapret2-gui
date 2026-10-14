import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Gauge,
  Play,
  Loader2,
  X,
  AlertTriangle,
  Tv,
  CheckCircle2,
  Timer,
  DownloadCloud,
} from "lucide-react";
import { SpeedtestResult } from "../types";
import { useI18n } from "../i18n";

interface SpeedtestModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const SpeedtestModal: React.FC<SpeedtestModalProps> = ({
  isOpen,
  onClose,
}) => {
  const { t } = useI18n();
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [result, setResult] = useState<SpeedtestResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen) return null;

  const handleStartSpeedtest = async () => {
    setIsRunning(true);
    setError(null);
    setResult(null);

    try {
      const res = await invoke<SpeedtestResult>("benchmark_youtube_speed");
      setResult(res);
    } catch (err: any) {
      console.error("YouTube speedtest failed:", err);
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsRunning(false);
    }
  };

  const getTierColor = (tier: string) => {
    if (tier.includes("4K")) return "text-emerald-400 bg-emerald-500/10 border-emerald-500/30";
    if (tier.includes("1080p")) return "text-blue-400 bg-blue-500/10 border-blue-500/30";
    if (tier.includes("720p")) return "text-amber-400 bg-amber-500/10 border-amber-500/30";
    return "text-rose-400 bg-rose-500/10 border-rose-500/30";
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div className="relative flex max-h-[90vh] w-full max-w-lg flex-col rounded-2xl border border-slate-800 bg-slate-900 shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between border-b border-slate-800 p-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-rose-600 to-red-600 shadow-md shadow-red-600/20">
              <Tv className="h-5 w-5 text-white" />
            </div>
            <div>
              <h2 className="text-base font-bold text-white">{t.speedtest.title}</h2>
              <p className="text-xs text-slate-400 mt-0.5">{t.speedtest.subtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-800 hover:text-white transition-colors cursor-pointer"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6 text-center">
          {error && (
            <div className="flex items-start gap-3 rounded-xl border border-rose-500/30 bg-rose-500/10 p-4 text-xs text-rose-300 text-left">
              <AlertTriangle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
              <div>
                <span className="font-semibold text-rose-200">Ошибка замера скорости:</span>
                <p className="mt-0.5 leading-relaxed">{error}</p>
              </div>
            </div>
          )}

          {/* Speed Display */}
          <div className="flex flex-col items-center justify-center py-4">
            <div className="relative flex h-36 w-36 items-center justify-center rounded-full border-4 border-slate-800 bg-slate-950/80 shadow-inner">
              {isRunning ? (
                <Loader2 className="h-12 w-12 animate-spin text-rose-500" />
              ) : result ? (
                <div className="flex flex-col items-center">
                  <span className="text-3xl font-extrabold tracking-tight text-white font-mono">
                    {result.speed_mbps.toFixed(1)}
                  </span>
                  <span className="text-xs font-semibold uppercase text-rose-400 mt-0.5">
                    Мбит/с
                  </span>
                </div>
              ) : (
                <Gauge className="h-12 w-12 text-slate-600" />
              )}
            </div>

            <p className="mt-4 text-xs text-slate-400 max-w-sm">
              {isRunning
                ? t.speedtest.running
                : result
                ? result.message
                : "Запустите тест для проверки реальной пропускной способности видеопотока"}
            </p>
          </div>

          {/* Metrics breakdown */}
          {result && (
            <div className="space-y-3 text-left">
              <div
                className={`flex items-center justify-between p-3.5 rounded-xl border font-semibold text-xs ${getTierColor(
                  result.resolution_tier
                )}`}
              >
                <div className="flex items-center gap-2">
                  <CheckCircle2 className="h-4 w-4 shrink-0" />
                  <span>{t.speedtest.resultVerdict}</span>
                </div>
                <span className="font-bold text-sm tracking-wide">
                  {result.resolution_tier}
                </span>
              </div>

              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="flex items-center gap-2.5 rounded-xl border border-slate-800 bg-slate-950/60 p-3">
                  <Timer className="h-4 w-4 text-cyan-400 shrink-0" />
                  <div>
                    <span className="text-[11px] text-slate-500">
                      {t.speedtest.resultDuration}
                    </span>
                    <div className="font-semibold text-slate-200 mt-0.5 font-mono">
                      {(result.duration_ms / 1000).toFixed(2)} с
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-2.5 rounded-xl border border-slate-800 bg-slate-950/60 p-3">
                  <DownloadCloud className="h-4 w-4 text-purple-400 shrink-0" />
                  <div>
                    <span className="text-[11px] text-slate-500">
                      {t.speedtest.resultPayload}
                    </span>
                    <div className="font-semibold text-slate-200 mt-0.5 font-mono">
                      {(result.bytes_downloaded / (1024 * 1024)).toFixed(1)} МБ
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Action button */}
          <div className="pt-2">
            <button
              onClick={handleStartSpeedtest}
              disabled={isRunning}
              className="w-full flex items-center justify-center gap-2.5 rounded-xl bg-gradient-to-r from-rose-600 to-red-600 py-3 font-semibold text-sm text-white shadow-lg shadow-rose-600/20 hover:from-rose-500 hover:to-red-500 active:scale-[0.98] transition-all cursor-pointer disabled:opacity-50"
            >
              {isRunning ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>{t.speedtest.running}</span>
                </>
              ) : (
                <>
                  <Play className="h-4 w-4 fill-white" />
                  <span>{t.speedtest.runBtn}</span>
                </>
              )}
            </button>
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end border-t border-slate-800 p-4">
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 hover:bg-slate-700 transition-colors cursor-pointer"
          >
            {t.speedtest.closeBtn}
          </button>
        </div>
      </div>
    </div>
  );
};
