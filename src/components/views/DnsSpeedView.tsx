import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe2,
  ShieldCheck,
  Check,
  Loader2,
  AlertCircle,
  Tv,
  Gauge,
  Play,
  Timer,
  DownloadCloud,
  CheckCircle2,
  RefreshCw,
  Zap,
} from "lucide-react";
import { DnsStatus, SpeedtestResult } from "../../types";
import { useI18n } from "../../i18n";

export const DnsSpeedView: React.FC = React.memo(() => {
  const { t } = useI18n();

  // DNS State
  const [dnsStatus, setDnsStatus] = useState<DnsStatus | null>(null);
  const [isLoadingDns, setIsLoadingDns] = useState<boolean>(false);
  const [applyingDnsId, setApplyingDnsId] = useState<string | null>(null);
  const [dnsMessage, setDnsMessage] = useState<string | null>(null);
  const [dnsError, setDnsError] = useState<string | null>(null);

  // Speedtest State
  const [isSpeedRunning, setIsSpeedRunning] = useState<boolean>(false);
  const [speedResult, setSpeedResult] = useState<SpeedtestResult | null>(null);
  const [speedError, setSpeedError] = useState<string | null>(null);

  useEffect(() => {
    loadDns();
  }, []);

  const loadDns = async () => {
    setIsLoadingDns(true);
    setDnsError(null);
    try {
      const res = await invoke<DnsStatus>("get_dns_status");
      setDnsStatus(res);
    } catch (err: any) {
      console.error("Failed to load DNS status:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsLoadingDns(false);
    }
  };

  const handleApplyDns = async (providerId: string) => {
    setApplyingDnsId(providerId);
    setDnsError(null);
    setDnsMessage(null);
    try {
      const updated = await invoke<DnsStatus>("set_system_dns", { providerId });
      setDnsStatus(updated);
      setDnsMessage(t.dns.successApplied);
      setTimeout(() => setDnsMessage(null), 4000);
    } catch (err: any) {
      console.error("Failed to set system DNS:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setApplyingDnsId(null);
    }
  };

  const handleStartSpeedtest = async () => {
    setIsSpeedRunning(true);
    setSpeedError(null);
    setSpeedResult(null);

    try {
      const res = await invoke<SpeedtestResult>("benchmark_youtube_speed");
      setSpeedResult(res);
    } catch (err: any) {
      console.error("YouTube speedtest failed:", err);
      setSpeedError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsSpeedRunning(false);
    }
  };

  const getTierColor = (tier: string) => {
    if (tier.includes("4K")) return "text-emerald-400 bg-emerald-500/10 border-emerald-500/30";
    if (tier.includes("1080p")) return "text-blue-400 bg-blue-500/10 border-blue-500/30";
    if (tier.includes("720p")) return "text-amber-400 bg-amber-500/10 border-amber-500/30";
    return "text-rose-400 bg-rose-500/10 border-rose-500/30";
  };

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 pb-6">
      {/* DNS Manager Section */}
      <div className="rounded-2xl border border-white/[0.08] bg-slate-900/60 p-5 backdrop-blur-xl space-y-5">
        <div className="flex items-center justify-between border-b border-white/[0.06] pb-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-cyan-500 to-blue-600 shadow-md shadow-cyan-500/20 text-white">
              <Globe2 className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-sm font-bold text-white">{t.dns.title}</h3>
              <p className="text-xs text-slate-400">{t.dns.subtitle}</p>
            </div>
          </div>
          <button
            onClick={loadDns}
            disabled={isLoadingDns}
            className="p-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02] text-slate-400 hover:text-white transition-all cursor-pointer disabled:opacity-50"
            title="Обновить текущий DNS"
          >
            <RefreshCw className={`h-4 w-4 ${isLoadingDns ? "animate-spin" : ""}`} />
          </button>
        </div>

        {dnsError && (
          <div className="flex items-start gap-2.5 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
            <span>{dnsError}</span>
          </div>
        )}

        {dnsMessage && (
          <div className="flex items-center gap-2 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs text-emerald-300">
            <CheckCircle2 className="h-4 w-4 text-emerald-400 shrink-0" />
            <span>{dnsMessage}</span>
          </div>
        )}

        {/* Current DNS Status Badge */}
        {dnsStatus && (
          <div className="rounded-xl border border-white/[0.06] bg-slate-950/60 p-3.5 space-y-1.5">
            <div className="flex items-center justify-between text-xs">
              <span className="text-slate-400">{t.dns.adapterLabel}:</span>
              <span className="font-semibold text-white truncate max-w-xs">
                {dnsStatus.adapter_name}
              </span>
            </div>
            <div className="flex items-center justify-between text-xs font-mono">
              <span className="text-slate-500">{t.dns.currentDnsLabel}:</span>
              <span className="text-cyan-400">
                {(dnsStatus.current_dns && dnsStatus.current_dns.length > 0)
                  ? dnsStatus.current_dns.join(", ")
                  : (dnsStatus.active_servers && dnsStatus.active_servers.length > 0)
                  ? dnsStatus.active_servers.join(", ")
                  : "DHCP (Автоматически)"}
              </span>
            </div>
          </div>
        )}

        {/* DNS Providers Grid */}
        <div className="space-y-2">
          {dnsStatus?.available_providers.map((p) => {
            const isCurrent = (dnsStatus.active_provider_id || dnsStatus.current_provider) === p.id;
            const isApplying = applyingDnsId === p.id;

            return (
              <div
                key={p.id}
                className={`flex items-center justify-between rounded-xl border p-3 transition-all ${
                  isCurrent
                    ? "border-cyan-500/40 bg-cyan-500/10 shadow-sm"
                    : "border-white/[0.05] bg-slate-950/40 hover:border-white/[0.1] hover:bg-slate-950/70"
                }`}
              >
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-bold text-white">{p.name}</span>
                    {isCurrent && (
                      <span className="rounded bg-cyan-500/20 px-1.5 py-0.5 text-[9px] font-bold text-cyan-300 uppercase">
                        {t.dns.activeBadge}
                      </span>
                    )}
                    {p.is_doh_capable && (
                      <span className="rounded bg-emerald-500/15 border border-emerald-500/30 px-1.5 py-0.5 text-[9px] font-semibold text-emerald-400">
                        {t.dns.dohBadge}
                      </span>
                    )}
                  </div>
                  <p className="text-[11px] text-slate-400 font-mono">
                    {p.primary ? `${p.primary} / ${p.secondary}` : "DHCP / Провайдер"}
                  </p>
                  <p className="text-[10px] text-slate-500 leading-tight">
                    {p.description}
                  </p>
                </div>

                <button
                  onClick={() => handleApplyDns(p.id)}
                  disabled={isCurrent || isApplying}
                  className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition-all cursor-pointer disabled:cursor-not-allowed shrink-0 ${
                    isCurrent
                      ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                      : "bg-white/[0.05] hover:bg-cyan-600 hover:text-white text-slate-300 border border-white/[0.08]"
                  }`}
                >
                  {isApplying ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-cyan-400" />
                  ) : isCurrent ? (
                    <>
                      <Check className="h-3.5 w-3.5" />
                      <span>{t.dns.activeBadge}</span>
                    </>
                  ) : (
                    <span>{t.dns.applyBtn}</span>
                  )}
                </button>
              </div>
            );
          })}
        </div>
      </div>

      {/* YouTube 4K Stream Speedtest Section */}
      <div className="rounded-2xl border border-white/[0.08] bg-slate-900/60 p-5 backdrop-blur-xl space-y-5">
        <div className="flex items-center justify-between border-b border-white/[0.06] pb-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-rose-600 to-red-600 shadow-md shadow-red-600/20 text-white">
              <Tv className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-sm font-bold text-white">{t.speedtest.title}</h3>
              <p className="text-xs text-slate-400">{t.speedtest.subtitle}</p>
            </div>
          </div>
        </div>

        {speedError && (
          <div className="flex items-start gap-2.5 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
            <span>{speedError}</span>
          </div>
        )}

        {/* Speed Dial / Gauge */}
        <div className="flex flex-col items-center justify-center py-4">
          <div className="relative flex h-36 w-36 items-center justify-center rounded-full border-4 border-slate-800 bg-slate-950/80 shadow-inner">
            {isSpeedRunning ? (
              <Loader2 className="h-10 w-10 animate-spin text-rose-500" />
            ) : speedResult ? (
              <div className="flex flex-col items-center">
                <span className="text-3xl font-extrabold tracking-tight text-white font-mono">
                  {(speedResult.speed_mbps ?? speedResult.mbps ?? 0).toFixed(1)}
                </span>
                <span className="text-[10px] font-semibold uppercase text-rose-400">Мбит/с</span>
              </div>
            ) : (
              <Gauge className="h-12 w-12 text-slate-600" />
            )}
          </div>

          <div className="mt-4">
            <button
              onClick={handleStartSpeedtest}
              disabled={isSpeedRunning}
              className="flex items-center gap-2 rounded-xl bg-gradient-to-r from-rose-600 to-red-600 hover:from-rose-500 hover:to-red-500 px-6 py-2.5 text-xs font-bold text-white shadow-lg shadow-rose-600/20 transition-all active:scale-95 cursor-pointer disabled:opacity-50"
            >
              {isSpeedRunning ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span>{t.speedtest.running}</span>
                </>
              ) : (
                <>
                  <Play className="h-4 w-4 fill-current" />
                  <span>{t.speedtest.runBtn}</span>
                </>
              )}
            </button>
          </div>
        </div>

        {/* Speedtest Results Card */}
        {speedResult && (
          <div className="space-y-3 rounded-xl border border-white/[0.06] bg-slate-950/60 p-4">
            <div className="flex items-center justify-between border-b border-white/[0.05] pb-2.5">
              <span className="text-xs text-slate-400">{t.speedtest.resultVerdict}:</span>
              <span
                className={`rounded-lg border px-2.5 py-0.5 text-xs font-bold ${getTierColor(
                  speedResult.resolution_tier || speedResult.quality_tier || ""
                )}`}
              >
                {speedResult.resolution_tier || speedResult.quality_tier || "N/A"}
              </span>
            </div>

            <div className="grid grid-cols-2 gap-3 text-xs">
              <div className="flex items-center gap-2">
                <Timer className="h-4 w-4 text-slate-500" />
                <span className="text-slate-400">{t.speedtest.resultDuration}:</span>
                <span className="font-mono font-semibold text-slate-200">
                  {((speedResult.duration_ms ?? speedResult.download_time_ms ?? 0) / 1000).toFixed(2)} с
                </span>
              </div>
              <div className="flex items-center gap-2">
                <DownloadCloud className="h-4 w-4 text-slate-500" />
                <span className="text-slate-400">{t.speedtest.resultPayload}:</span>
                <span className="font-mono font-semibold text-slate-200">
                  {(speedResult.bytes_downloaded / (1024 * 1024)).toFixed(1)} МБ
                </span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
});
