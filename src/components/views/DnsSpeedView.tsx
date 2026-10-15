import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe2,
  Check,
  Loader2,
  AlertCircle,
  Tv,
  Play,
  Timer,
  DownloadCloud,
  CheckCircle2,
  RefreshCw,
  Activity,
  ShieldCheck,
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
  const [dnsScope, setDnsScope] = useState<"full" | "split">("full");

  // Speedtest State
  const [isSpeedRunning, setIsSpeedRunning] = useState<boolean>(false);
  const [speedProgress, setSpeedProgress] = useState<number>(0);
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
      if (res.dns_scope) {
        setDnsScope(res.dns_scope);
      }
    } catch (err: any) {
      console.error("Failed to load DNS status:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsLoadingDns(false);
    }
  };

  const handleApplyDns = async (providerId: string, scopeToUse?: "full" | "split") => {
    setApplyingDnsId(providerId);
    setDnsError(null);
    setDnsMessage(null);
    const targetScope = scopeToUse || dnsScope;
    try {
      const updated = await invoke<DnsStatus>("set_system_dns", {
        providerId,
        scope: targetScope,
      });
      setDnsStatus(updated);
      if (updated.dns_scope) {
        setDnsScope(updated.dns_scope);
      }
      setDnsMessage(t.dns.successApplied);
      setTimeout(() => setDnsMessage(null), 4000);
    } catch (err: any) {
      console.error("Failed to set system DNS:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setApplyingDnsId(null);
    }
  };

  const handleScopeChange = async (newScope: "full" | "split") => {
    setDnsScope(newScope);
    const currentId = dnsStatus?.active_provider_id || dnsStatus?.current_provider;
    if (currentId && currentId !== "system" && currentId !== "dhcp") {
      await handleApplyDns(currentId, newScope);
    }
  };

  const handleStartSpeedtest = async () => {
    setIsSpeedRunning(true);
    setSpeedProgress(15);
    setSpeedError(null);
    setSpeedResult(null);

    const interval = setInterval(() => {
      setSpeedProgress((prev) => {
        if (prev >= 90) return prev;
        return prev + Math.floor(Math.random() * 15) + 5;
      });
    }, 350);

    try {
      const res = await invoke<SpeedtestResult>("benchmark_youtube_speed");
      setSpeedProgress(100);
      setSpeedResult(res);
    } catch (err: any) {
      console.error("YouTube speedtest failed:", err);
      setSpeedError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      clearInterval(interval);
      setIsSpeedRunning(false);
    }
  };

  const getTierBadge = (speedMbps: number) => {
    if (speedMbps >= 25) {
      return {
        label: "4K Ultra HD (2160p 60fps)",
        badgeClass: "text-emerald-400 bg-emerald-500/10 border-emerald-500/30",
        desc: "Идеально: моментальная буферизация 4K HDR",
      };
    }
    if (speedMbps >= 15) {
      return {
        label: "2K Quad HD (1440p)",
        badgeClass: "text-blue-400 bg-blue-500/10 border-blue-500/30",
        desc: "Отлично: быстрая загрузка 1440p и 1080p60",
      };
    }
    if (speedMbps >= 6) {
      return {
        label: "Full HD (1080p)",
        badgeClass: "text-amber-400 bg-amber-500/10 border-amber-500/30",
        desc: "Хорошо: стабильное воспроизведение 1080p",
      };
    }
    return {
      label: "HD (720p / 480p)",
      badgeClass: "text-rose-400 bg-rose-500/10 border-rose-500/30",
      desc: "Низкая скорость: возможны задержки в 1080p",
    };
  };

  const speedValue = speedResult
    ? (speedResult.speed_mbps ?? speedResult.mbps ?? 0)
    : 0;
  const currentTier = speedResult ? getTierBadge(speedValue) : null;

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 pb-4">
      {/* DNS Manager Section */}
      <div className="rounded-2xl border border-[#212635] bg-[#141720] p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-[#202534] pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-blue-500/20 to-blue-600/5 border border-blue-500/25 text-blue-400 shadow-sm shadow-blue-500/5">
              <Globe2 className="h-4 w-4 shrink-0" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-white">{t.dns.title}</h3>
              <p className="text-[11px] text-slate-400">{t.dns.subtitle}</p>
            </div>
          </div>
          <button
            type="button"
            onClick={loadDns}
            disabled={isLoadingDns}
            className="p-1.5 rounded-lg border border-[#252b3b] bg-[#161a22] text-slate-400 hover:text-white hover:bg-[#1b212c] transition-all cursor-pointer disabled:opacity-50 shrink-0"
            title="Обновить текущий DNS"
          >
            <RefreshCw className={`h-3.5 w-3.5 shrink-0 ${isLoadingDns ? "animate-spin" : ""}`} />
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

        {/* DNS Scope Selector (Whole Windows vs Selective Split-DNS) */}
        <div className="rounded-xl border border-[#202534] bg-[#10131a] p-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-medium text-slate-300">{t.dns.scopeLabel}</span>
            <span className={`text-[10px] font-semibold px-2 py-0.5 rounded-full font-mono ${
              dnsStatus?.split_dns_active
                ? "bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                : "bg-blue-500/15 border border-blue-500/30 text-blue-300"
            }`}>
              {dnsStatus?.split_dns_active ? t.dns.splitActiveBadge : "ADAPTER"}
            </span>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => handleScopeChange("full")}
              className={`flex flex-col text-left p-2.5 rounded-lg border transition-all cursor-pointer ${
                dnsScope === "full"
                  ? "border-blue-500/50 bg-blue-500/15 text-white ring-1 ring-blue-500/30 shadow-sm"
                  : "border-[#1e2432] bg-[#141720] text-slate-400 hover:text-slate-200 hover:bg-[#181d28]"
              }`}
            >
              <span className="text-xs font-semibold">{t.dns.scopeFullTitle}</span>
              <span className="text-[10px] text-slate-400 mt-1 leading-tight">{t.dns.scopeFullDesc}</span>
            </button>
            <button
              type="button"
              onClick={() => handleScopeChange("split")}
              className={`flex flex-col text-left p-2.5 rounded-lg border transition-all cursor-pointer ${
                dnsScope === "split"
                  ? "border-emerald-500/50 bg-emerald-500/15 text-white ring-1 ring-emerald-500/30 shadow-sm"
                  : "border-[#1e2432] bg-[#141720] text-slate-400 hover:text-slate-200 hover:bg-[#181d28]"
              }`}
            >
              <div className="flex items-center gap-1.5">
                <span className="text-xs font-semibold">{t.dns.scopeSplitTitle}</span>
              </div>
              <span className="text-[10px] text-slate-400 mt-1 leading-tight">{t.dns.scopeSplitDesc}</span>
            </button>
          </div>
        </div>

        {/* Educational Scope & Synergy Card */}
        <div className="rounded-xl border border-blue-500/20 bg-blue-500/[0.05] p-3.5 space-y-2 text-xs">
          <div className="flex items-center gap-2 text-blue-300 font-medium">
            <ShieldCheck className="h-4 w-4 text-blue-400 shrink-0" />
            <span>{t.dns.scopeBannerTitle}</span>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-[11px] text-slate-300">
            <div className="rounded-lg bg-[#10131a]/80 border border-white/[0.04] p-2 space-y-1">
              <span className="font-semibold text-slate-200 block">{t.dns.scopeBannerScope}</span>
              <p className="text-slate-400 leading-relaxed">{t.dns.scopeBannerScopeDesc}</p>
            </div>
            <div className="rounded-lg bg-[#10131a]/80 border border-white/[0.04] p-2 space-y-1">
              <span className="font-semibold text-slate-200 block">{t.dns.scopeBannerSynergy}</span>
              <p className="text-slate-400 leading-relaxed">{t.dns.scopeBannerSynergyDesc}</p>
            </div>
          </div>

          <p className="text-[10px] text-blue-300/80 italic pt-0.5">
            💡 {t.dns.scopeBannerHint}
          </p>
        </div>

        {/* Current DNS Status Card */}
        {dnsStatus && (
          <div className="rounded-xl border border-[#202534] bg-[#12151d] p-3 space-y-1.5">
            <div className="flex items-center justify-between text-xs">
              <span className="text-slate-400">{t.dns.adapterLabel}</span>
              <span className="font-semibold text-white truncate max-w-xs font-mono text-[11px]">
                {dnsStatus.adapter_name}
              </span>
            </div>
            <div className="flex items-center justify-between text-xs">
              <span className="text-slate-400">{t.dns.currentDnsLabel}</span>
              <span className={`font-mono text-[11px] font-semibold ${
                dnsStatus.split_dns_active ? "text-emerald-400" : "text-blue-400"
              }`}>
                {dnsStatus.split_dns_active
                  ? `Split NRPT: ${dnsStatus.split_dns_servers?.join(", ") || dnsStatus.current_dns.join(", ")}`
                  : dnsStatus.current_dns && dnsStatus.current_dns.length > 0
                  ? dnsStatus.current_dns.join(", ")
                  : dnsStatus.active_servers && dnsStatus.active_servers.length > 0
                  ? dnsStatus.active_servers.join(", ")
                  : "DHCP (Автоматически)"}
              </span>
            </div>
          </div>
        )}

        {/* DNS Providers List */}
        <div className="space-y-2">
          {dnsStatus?.available_providers.map((p) => {
            const isCurrent = (dnsStatus.active_provider_id || dnsStatus.current_provider) === p.id;
            const isApplying = applyingDnsId === p.id;

            return (
              <div
                key={p.id}
                className={`flex items-center justify-between rounded-xl border p-3 transition-colors ${
                  isCurrent
                    ? "border-[#384359] bg-[#1d2332]"
                    : "border-[#202534] bg-[#12151d] hover:border-[#2b3345] hover:bg-[#161a23]"
                }`}
              >
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-semibold text-white">{p.name}</span>
                    {isCurrent && (
                      <span className={`rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase ${
                        dnsStatus.split_dns_active
                          ? "bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
                          : "bg-blue-500/20 text-blue-300"
                      }`}>
                        {dnsStatus.split_dns_active ? t.dns.splitActiveBadge : t.dns.activeBadge}
                      </span>
                    )}
                    {p.is_doh_capable && (
                      <span className="rounded bg-emerald-500/15 border border-emerald-500/30 px-1.5 py-0.5 text-[9px] font-medium text-emerald-400">
                        {t.dns.dohBadge}
                      </span>
                    )}
                  </div>
                  <p className="text-[11px] text-slate-400 font-mono">
                    {p.primary ? `${p.primary} / ${p.secondary}` : "DHCP (Провайдер)"}
                  </p>
                  <p className="text-[10px] text-slate-500 leading-tight">
                    {p.description}
                  </p>
                </div>

                <button
                  type="button"
                  onClick={() => handleApplyDns(p.id)}
                  disabled={isCurrent || isApplying}
                  className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-semibold transition-all cursor-pointer disabled:cursor-not-allowed shrink-0 ${
                    isCurrent
                      ? dnsStatus.split_dns_active
                        ? "bg-emerald-500/15 text-emerald-300 border border-emerald-500/30"
                        : "bg-blue-500/15 text-blue-300 border border-blue-500/30"
                      : "bg-[#181d28] hover:bg-[#2563eb] hover:text-white text-slate-300 border border-[#2b3345]"
                  }`}
                >
                  {isApplying ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-400" />
                  ) : isCurrent ? (
                    <>
                      <Check className="h-3.5 w-3.5" />
                      <span>{dnsStatus.split_dns_active ? t.dns.splitActiveBadge : t.dns.activeBadge}</span>
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
      <div className="rounded-2xl border border-[#212635] bg-[#141720] p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-[#202534] pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-red-500/20 to-red-600/5 border border-red-500/25 text-red-400 shadow-sm shadow-red-500/5">
              <Tv className="h-4 w-4 shrink-0" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-white">{t.speedtest.title}</h3>
              <p className="text-[11px] text-slate-400">{t.speedtest.subtitle}</p>
            </div>
          </div>
        </div>

        {speedError && (
          <div className="flex items-start gap-2.5 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
            <span>{speedError}</span>
          </div>
        )}

        {/* Digital Benchmark Display */}
        <div className="rounded-xl border border-[#202534] bg-[#12151d] p-5 flex flex-col items-center justify-center space-y-4">
          <div className="flex items-center gap-2 text-xs text-slate-400">
            <Activity className="h-3.5 w-3.5 text-red-400" />
            <span>Целевой узел: YouTube Video Edge Cache (googlevideo.com)</span>
          </div>

          {/* Speed Value & Live Pulse */}
          <div className="flex flex-col items-center justify-center py-2">
            {isSpeedRunning ? (
              <div className="flex flex-col items-center space-y-2">
                <Loader2 className="h-8 w-8 animate-spin text-red-400" />
                <span className="text-xs font-medium text-slate-300">
                  Замер пропускной способности потока...
                </span>
                {/* Progress bar */}
                <div className="w-56 h-1.5 bg-[#1e2330] rounded-full overflow-hidden">
                  <div
                    className="h-full bg-red-500 transition-all duration-300 rounded-full"
                    style={{ width: `${speedProgress}%` }}
                  />
                </div>
              </div>
            ) : speedResult ? (
              <div className="flex flex-col items-center">
                <div className="flex items-baseline gap-1.5">
                  <span className="text-4xl font-extrabold tracking-tight text-white font-mono">
                    {speedValue.toFixed(1)}
                  </span>
                  <span className="text-xs font-bold uppercase text-red-400 font-mono">
                    Мбит/с
                  </span>
                </div>
                {currentTier && (
                  <span
                    className={`mt-2 rounded-md border px-2.5 py-0.5 text-xs font-semibold ${currentTier.badgeClass}`}
                  >
                    {currentTier.label}
                  </span>
                )}
              </div>
            ) : (
              <div className="flex flex-col items-center text-center py-2 space-y-1">
                <span className="text-2xl font-bold font-mono text-slate-500">—</span>
                <span className="text-xs text-slate-400">
                  Нажмите кнопку ниже для реального замера скорости YouTube
                </span>
              </div>
            )}
          </div>

          {/* Action Button */}
          <button
            type="button"
            onClick={handleStartSpeedtest}
            disabled={isSpeedRunning}
            className="flex items-center gap-2 rounded-xl bg-[#dc2626] hover:bg-[#b91c1c] px-6 py-2.5 text-xs font-semibold text-white shadow-sm transition-all active:scale-[0.98] cursor-pointer disabled:opacity-50"
          >
            {isSpeedRunning ? (
              <>
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                <span>{t.speedtest.running}</span>
              </>
            ) : (
              <>
                <Play className="h-3.5 w-3.5 fill-current" />
                <span>{t.speedtest.runBtn}</span>
              </>
            )}
          </button>
        </div>

        {/* Speedtest Metrics Cards */}
        {speedResult && (
          <div className="space-y-2 rounded-xl border border-[#202534] bg-[#12151d] p-3.5">
            <div className="grid grid-cols-2 gap-3 text-xs">
              <div className="flex items-center gap-2">
                <Timer className="h-4 w-4 text-slate-500" />
                <span className="text-slate-400">{t.speedtest.resultDuration}</span>
                <span className="font-mono font-semibold text-slate-200">
                  {((speedResult.duration_ms ?? speedResult.download_time_ms ?? 0) / 1000).toFixed(2)} с
                </span>
              </div>
              <div className="flex items-center gap-2">
                <DownloadCloud className="h-4 w-4 text-slate-500" />
                <span className="text-slate-400">{t.speedtest.resultPayload}</span>
                <span className="font-mono font-semibold text-slate-200">
                  {(speedResult.bytes_downloaded / (1024 * 1024)).toFixed(1)} МБ
                </span>
              </div>
            </div>
            {currentTier && (
              <div className="pt-2 border-t border-[#1c212d] text-[11px] text-slate-400 flex items-center gap-1.5">
                <ShieldCheck className="h-3.5 w-3.5 text-emerald-400 shrink-0" />
                <span>{currentTier.desc}</span>
              </div>
            )}
          </div>
        )}

        {/* Speed Requirements Helper Card */}
        <div className="rounded-xl border border-[#1f2431] bg-[#12141c] p-3 space-y-1.5 text-[11px]">
          <span className="font-semibold text-slate-300">Требования к битрейту YouTube:</span>
          <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-slate-400">
            <div>• 4K 2160p60: <span className="font-mono text-emerald-400 font-semibold">≥ 25 Мбит/с</span></div>
            <div>• 2K 1440p60: <span className="font-mono text-blue-400 font-semibold">≥ 15 Мбит/с</span></div>
            <div>• Full HD 1080p: <span className="font-mono text-amber-400 font-semibold">≥ 6 Мбит/с</span></div>
            <div>• HD 720p: <span className="font-mono text-slate-400 font-semibold">≥ 3 Мбит/с</span></div>
          </div>
        </div>
      </div>
    </div>
  );
});
