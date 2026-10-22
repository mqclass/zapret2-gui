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
  Trash2,
  Plus,
  FileText,
  Clock,
} from "lucide-react";
import { DnsStatus, SpeedtestResult, SmartHostsStatus } from "../../types";
import { useI18n } from "../../i18n";
import { DnsSplitModal } from "../DnsSplitModal";

export const DnsSpeedView: React.FC = React.memo(() => {
  const { t } = useI18n();

  // DNS State
  const [dnsStatus, setDnsStatus] = useState<DnsStatus | null>(null);
  const [isLoadingDns, setIsLoadingDns] = useState<boolean>(false);
  const [applyingDnsId, setApplyingDnsId] = useState<string | null>(null);
  const [dnsMessage, setDnsMessage] = useState<string | null>(null);
  const [dnsError, setDnsError] = useState<string | null>(null);
  const [dnsScope, setDnsScope] = useState<"full" | "split">("full");

  // Custom DNS form state
  const [customName, setCustomName] = useState("");
  const [customPrimary, setCustomPrimary] = useState("");
  const [customSecondary, setCustomSecondary] = useState("");
  const [isAddingCustom, setIsAddingCustom] = useState(false);
  const [isDnsSplitOpen, setIsDnsSplitOpen] = useState(false);

  // Speedtest State
  const [isSpeedRunning, setIsSpeedRunning] = useState<boolean>(false);
  const [speedProgress, setSpeedProgress] = useState<number>(0);
  const [speedResult, setSpeedResult] = useState<SpeedtestResult | null>(null);
  const [speedError, setSpeedError] = useState<string | null>(null);

  // Smart hosts state
  const [hostsStatus, setHostsStatus] = useState<SmartHostsStatus | null>(null);
  const [hostsBusy, setHostsBusy] = useState<boolean>(false);
  const [hostsBusyOp, setHostsBusyOp] = useState<"toggle" | "refresh" | "sources" | null>(null);
  const [hostsError, setHostsError] = useState<string | null>(null);
  const hostsTimers = React.useRef<number[]>([]);

  useEffect(() => {
    return () => {
      hostsTimers.current.forEach((id) => window.clearTimeout(id));
      hostsTimers.current = [];
    };
  }, []);

  const flashHostsMessage = (msg: string) => {
    setDnsMessage(msg);
    const id = window.setTimeout(() => setDnsMessage(null), 4000);
    hostsTimers.current.push(id);
  };

  useEffect(() => {
    loadDns();
    loadSmartHosts();
  }, []);

  const loadSmartHosts = async () => {
    try {
      const res = await invoke<SmartHostsStatus>("get_smart_hosts_status");
      setHostsStatus(res);
    } catch (err: any) {
      console.error("Failed to load smart hosts status:", err);
      setHostsError(typeof err === "string" ? err : JSON.stringify(err));
    }
  };

  const handleToggleSmartHosts = async () => {
    setHostsBusy(true);
    setHostsBusyOp("toggle");
    setHostsError(null);
    setDnsMessage(null);
    try {
      const res = await invoke<SmartHostsStatus>(
        hostsStatus?.enabled ? "disable_smart_hosts" : "enable_smart_hosts"
      );
      setHostsStatus(res);
      flashHostsMessage(hostsStatus?.enabled ? t.smartHosts.successDisabled : t.smartHosts.successEnabled);
    } catch (err: any) {
      setHostsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setHostsBusy(false);
      setHostsBusyOp(null);
    }
  };

  const handleRefreshSmartHosts = async () => {
    setHostsBusy(true);
    setHostsBusyOp("refresh");
    setHostsError(null);
    setDnsMessage(null);
    try {
      const res = await invoke<SmartHostsStatus>("refresh_smart_hosts");
      setHostsStatus(res);
      flashHostsMessage(t.smartHosts.successRefreshed);
    } catch (err: any) {
      setHostsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setHostsBusy(false);
      setHostsBusyOp(null);
    }
  };

  const handleToggleSource = async (sourceId: string) => {
    const current = hostsStatus?.sources ?? [];
    const next = current
      .filter((s) => (s.id === sourceId ? !s.enabled : s.enabled))
      .map((s) => s.id);
    setHostsBusy(true);
    setHostsBusyOp("sources");
    setHostsError(null);
    try {
      const res = await invoke<SmartHostsStatus>("set_smart_hosts_sources", { sources: next });
      setHostsStatus(res);
      if (!res.enabled) {
        flashHostsMessage(t.smartHosts.sourcesSavedHint);
      }
    } catch (err: any) {
      setHostsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setHostsBusy(false);
      setHostsBusyOp(null);
    }
  };

  const handleToggleAdblock = async () => {
    setHostsBusy(true);
    setHostsBusyOp("sources");
    setHostsError(null);
    try {
      const res = await invoke<SmartHostsStatus>("set_smart_hosts_adblock", {
        include: !(hostsStatus?.include_adblock ?? false),
      });
      setHostsStatus(res);
      if (!res.enabled) {
        flashHostsMessage(t.smartHosts.sourcesSavedHint);
      }
    } catch (err: any) {
      setHostsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setHostsBusy(false);
      setHostsBusyOp(null);
    }
  };

  const formatHostsDate = (ts: number | null) => {
    if (!ts) return t.smartHosts.neverUpdated;
    return new Date(ts * 1000).toLocaleString();
  };

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

  const handleApplyDns = async (providerId: string, scopeToUse?: "full" | "split"): Promise<boolean> => {
    setApplyingDnsId(providerId);
    setDnsError(null);
    setDnsMessage(null);
    const targetScope = scopeToUse || dnsScope;
    try {
      const updated = await invoke<DnsStatus>("set_system_dns", {
        provider_id: providerId,
        scope: targetScope,
      });
      setDnsStatus(updated);
      if (updated.dns_scope) {
        setDnsScope(updated.dns_scope);
      }
      setDnsMessage(t.dns.successApplied);
      setTimeout(() => setDnsMessage(null), 4000);
      return true;
    } catch (err: any) {
      console.error("Failed to set system DNS:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
      return false;
    } finally {
      setApplyingDnsId(null);
    }
  };

  const handleScopeChange = async (newScope: "full" | "split") => {
    const prevScope = dnsScope;
    setDnsScope(newScope);
    const currentId = dnsStatus?.active_provider_id || dnsStatus?.current_provider;
    if (currentId && currentId !== "system" && currentId !== "dhcp") {
      // Roll back the optimistic scope switch if the apply fails.
      const ok = await handleApplyDns(currentId, newScope);
      if (!ok) {
        setDnsScope(prevScope);
      }
    }
  };

  const handleAddCustomDns = async () => {
    setIsAddingCustom(true);
    setDnsError(null);
    setDnsMessage(null);
    try {
      const updated = await invoke<DnsStatus>("add_custom_dns", {
        name: customName,
        primary: customPrimary,
        secondary: customSecondary,
      });
      setDnsStatus(updated);
      setCustomName("");
      setCustomPrimary("");
      setCustomSecondary("");
      setDnsMessage(t.dns.successApplied);
      setTimeout(() => setDnsMessage(null), 4000);
    } catch (err: any) {
      console.error("Failed to add custom DNS:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsAddingCustom(false);
    }
  };

  const handleRemoveCustomDns = async (providerId: string) => {
    // providerId is "custom:<id>"; strip the prefix for the backend.
    const id = providerId.startsWith("custom:") ? providerId.slice(7) : providerId;
    setDnsError(null);
    setDnsMessage(null);
    try {
      const updated = await invoke<DnsStatus>("remove_custom_dns", { id });
      setDnsStatus(updated);
    } catch (err: any) {
      console.error("Failed to remove custom DNS:", err);
      setDnsError(typeof err === "string" ? err : JSON.stringify(err));
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
        desc: t.speedtest.tier4kDesc,
      };
    }
    if (speedMbps >= 15) {
      return {
        label: "2K Quad HD (1440p)",
        badgeClass: "text-accent bg-accent-wash border-accent-line",
        desc: t.speedtest.tier2kDesc,
      };
    }
    if (speedMbps >= 6) {
      return {
        label: "Full HD (1080p)",
        badgeClass: "text-amber-400 bg-amber-500/10 border-amber-500/30",
        desc: t.speedtest.tierFhdDesc,
      };
    }
    return {
      label: "HD (720p / 480p)",
      badgeClass: "text-rose-400 bg-rose-500/10 border-rose-500/30",
      desc: t.speedtest.tierHdDesc,
    };
  };

  const speedValue = speedResult
    ? (speedResult.speed_mbps ?? speedResult.mbps ?? 0)
    : 0;
  const currentTier = speedResult ? getTierBadge(speedValue) : null;

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 pb-4">
      {/* DNS Manager Section */}
      <div className="t-card p-5 space-y-4 stagger-item" style={{ "--stagger-i": 0 } as React.CSSProperties}>
        <div className="flex items-center justify-between border-b border-line pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-blue-500/20 to-accent-fill/5 border border-accent-line text-accent shadow-sm shadow-blue-500/5">
              <Globe2 className="h-4 w-4 shrink-0" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-ink">{t.dns.title}</h3>
              <p className="text-[11px] text-ink4">{t.dns.subtitle}</p>
            </div>
          </div>
          <button
            type="button"
            onClick={loadDns}
            disabled={isLoadingDns}
            className="p-1.5 rounded-control border border-line bg-raise text-ink4 hover:text-ink hover:bg-hovert transition-all cursor-pointer disabled:opacity-50 shrink-0"
            title={t.dns.refreshCurrentTitle}
          >
            <RefreshCw className={`h-3.5 w-3.5 shrink-0 ${isLoadingDns ? "animate-spin" : ""}`} />
          </button>
        </div>

        {dnsError && (
          <div className="flex items-start gap-2.5 rounded-panel border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
            <span>{dnsError}</span>
          </div>
        )}

        {dnsMessage && (
          <div className="flex items-center gap-2 rounded-panel border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs text-emerald-300">
            <CheckCircle2 className="h-4 w-4 text-emerald-400 shrink-0" />
            <span>{dnsMessage}</span>
          </div>
        )}

        {/* DNS Scope Selector (Whole Windows vs Selective Split-DNS) */}
        <div className="rounded-panel border border-line bg-hovert p-3 space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-medium text-ink3">{t.dns.scopeLabel}</span>
            <div className="flex items-center gap-2">
              {dnsStatus?.split_dns_active && (
                <button
                  type="button"
                  onClick={() => setIsDnsSplitOpen(true)}
                  className="text-[10px] font-semibold px-2 py-0.5 rounded-full bg-hovert border border-line text-ink3 hover:text-accent hover:border-accent-line transition-all cursor-pointer"
                >
                  {t.dnsSplit.configure}
                </button>
              )}
              <span className={`text-[10px] font-semibold px-2 py-0.5 rounded-full font-mono ${
                dnsStatus?.split_dns_active
                  ? "bg-emerald-500/15 border border-emerald-500/30 text-emerald-300"
                  : "bg-accent-wash border border-accent-line text-accent-soft"
              }`}>
                {dnsStatus?.split_dns_active ? t.dns.splitActiveBadge : "ADAPTER"}
              </span>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => handleScopeChange("full")}
              className={`flex flex-col text-left p-2.5 rounded-control border transition-all cursor-pointer ${
                dnsScope === "full"
                  ? "border-accent-line bg-accent-wash text-ink ring-1 ring-accent-line shadow-sm"
                  : "border-line bg-card text-ink4 hover:text-ink2 hover:bg-hovert"
              }`}
            >
              <span className="text-xs font-semibold">{t.dns.scopeFullTitle}</span>
              <span className="text-[10px] text-ink4 mt-1 leading-tight">{t.dns.scopeFullDesc}</span>
            </button>
            <button
              type="button"
              onClick={() => handleScopeChange("split")}
              className={`flex flex-col text-left p-2.5 rounded-control border transition-all cursor-pointer ${
                dnsScope === "split"
                  ? "border-emerald-500/50 bg-emerald-500/15 text-ink ring-1 ring-emerald-500/30 shadow-sm"
                  : "border-line bg-card text-ink4 hover:text-ink2 hover:bg-hovert"
              }`}
            >
              <div className="flex items-center gap-1.5">
                <span className="text-xs font-semibold">{t.dns.scopeSplitTitle}</span>
              </div>
              <span className="text-[10px] text-ink4 mt-1 leading-tight">{t.dns.scopeSplitDesc}</span>
            </button>
          </div>
        </div>

        {/* Educational Scope & Synergy Card */}
        <div className="rounded-panel border border-accent-line bg-accent/[0.05] p-3.5 space-y-2 text-xs">
          <div className="flex items-center gap-2 text-accent-soft font-medium">
            <ShieldCheck className="h-4 w-4 text-accent shrink-0" />
            <span>{t.dns.scopeBannerTitle}</span>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-[11px] text-ink3">
            <div className="rounded-control bg-cardin border border-line p-2 space-y-1">
              <span className="font-semibold text-ink2 block">{t.dns.scopeBannerScope}</span>
              <p className="text-ink4 leading-relaxed">{t.dns.scopeBannerScopeDesc}</p>
            </div>
            <div className="rounded-control bg-cardin border border-line p-2 space-y-1">
              <span className="font-semibold text-ink2 block">{t.dns.scopeBannerSynergy}</span>
              <p className="text-ink4 leading-relaxed">{t.dns.scopeBannerSynergyDesc}</p>
            </div>
          </div>

          <p className="text-[10px] text-accent-soft italic pt-0.5">
            💡 {t.dns.scopeBannerHint}
          </p>
        </div>

        {/* Current DNS Status Card */}
        {dnsStatus && (
          <div className="rounded-panel border border-line bg-cardin p-3 space-y-1.5">
            <div className="flex items-center justify-between text-xs">
              <span className="text-ink4">{t.dns.adapterLabel}</span>
              <span className="font-semibold text-ink truncate max-w-xs font-mono text-[11px]">
                {dnsStatus.adapter_name}
              </span>
            </div>
            <div className="flex items-center justify-between text-xs">
              <span className="text-ink4">{t.dns.currentDnsLabel}</span>
              <span className={`font-mono text-[11px] font-semibold ${
                dnsStatus.split_dns_active ? "text-emerald-400" : "text-accent"
              }`}>
                {dnsStatus.split_dns_active
                  ? `Split NRPT: ${dnsStatus.split_dns_servers?.join(", ") || dnsStatus.current_dns.join(", ")}`
                  : dnsStatus.current_dns && dnsStatus.current_dns.length > 0
                  ? dnsStatus.current_dns.join(", ")
                  : dnsStatus.active_servers && dnsStatus.active_servers.length > 0
                  ? dnsStatus.active_servers.join(", ")
                  : t.dns.dhcpAuto}
              </span>
            </div>
          </div>
        )}

        {/* DNS Providers List */}
        <div className="space-y-2">
          {dnsStatus?.available_providers?.map((p) => {
            const isCurrent = (dnsStatus.active_provider_id || dnsStatus.current_provider) === p.id;
            const isApplying = applyingDnsId === p.id;

            return (
              <div
                key={p.id}
                className={`flex items-center justify-between rounded-panel border p-3 transition-colors hover-lift ${
                  isCurrent
                    ? "border-linestrong bg-hovert"
                    : "border-line bg-cardin hover:border-line hover:bg-raise"
                }`}
              >
                <div className="space-y-0.5">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-semibold text-ink">{p.name}</span>
                    {isCurrent && (
                      <span className={`rounded-chip px-1.5 py-0.5 text-[9px] font-semibold uppercase ${
                        dnsStatus.split_dns_active
                          ? "bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
                          : "bg-accent-wash text-accent-soft"
                      }`}>
                        {dnsStatus.split_dns_active ? t.dns.splitActiveBadge : t.dns.activeBadge}
                      </span>
                    )}
                    {p.is_doh_capable && (
                      <span className="rounded-chip bg-emerald-500/15 border border-emerald-500/30 px-1.5 py-0.5 text-[9px] font-medium text-emerald-400">
                        {t.dns.dohBadge}
                      </span>
                    )}
                  </div>
                  <p className="text-[11px] text-ink4 font-mono">
                    {p.primary ? `${p.primary} / ${p.secondary}` : t.dns.dhcpProvider}
                  </p>
                  <p className="text-[10px] text-ink5 leading-tight">
                    {p.description}
                  </p>
                </div>

                <div className="flex items-center gap-2 shrink-0">
                  {p.id.startsWith("custom:") && (
                    <button
                      type="button"
                      onClick={() => handleRemoveCustomDns(p.id)}
                      title={t.dns.customDeleteTitle}
                      className="flex items-center justify-center rounded-control p-1.5 text-ink5 hover:text-rose-400 hover:bg-rose-500/10 border border-transparent hover:border-rose-500/30 transition-all cursor-pointer"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => handleApplyDns(p.id)}
                    disabled={isCurrent || isApplying}
                    className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold transition-all cursor-pointer disabled:cursor-not-allowed shrink-0 ${
                      isCurrent
                        ? dnsStatus.split_dns_active
                          ? "bg-emerald-500/15 text-emerald-300 border border-emerald-500/30"
                          : "bg-accent-wash text-accent-soft border border-accent-line"
                        : "bg-hovert hover:bg-accent-fill hover:text-accent-contrast text-ink3 border border-line"
                    }`}
                  >
                    {isApplying ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin text-accent" />
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
              </div>
            );
          })}
        </div>

        {/* Custom DNS form */}
        <div className="rounded-panel border border-line bg-cardin p-4 space-y-3">
          <div>
            <h4 className="text-xs font-semibold text-ink">{t.dns.customTitle}</h4>
            <p className="text-[11px] text-ink4">{t.dns.customSubtitle}</p>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            <div className="space-y-1">
              <label className="text-[10px] font-medium text-ink4 min-h-8 flex items-end">{t.dns.customNameLabel}</label>
              <input
                type="text"
                value={customName}
                onChange={(e) => setCustomName(e.target.value)}
                placeholder={t.dns.customNamePlaceholder}
                maxLength={60}
                className="w-full rounded-control border border-line bg-card px-2.5 py-1.5 text-xs text-ink placeholder:text-ink5 focus:border-accent-line focus:outline-none"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] font-medium text-ink4 min-h-8 flex items-end">{t.dns.customPrimaryLabel}</label>
              <input
                type="text"
                value={customPrimary}
                onChange={(e) => setCustomPrimary(e.target.value)}
                placeholder={t.dns.customPrimaryPlaceholder}
                inputMode="decimal"
                className="w-full rounded-control border border-line bg-card px-2.5 py-1.5 text-xs text-ink placeholder:text-ink5 font-mono focus:border-accent-line focus:outline-none"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] font-medium text-ink4 min-h-8 flex items-end">{t.dns.customSecondaryLabel}</label>
              <input
                type="text"
                value={customSecondary}
                onChange={(e) => setCustomSecondary(e.target.value)}
                placeholder={t.dns.customSecondaryPlaceholder}
                inputMode="decimal"
                className="w-full rounded-control border border-line bg-card px-2.5 py-1.5 text-xs text-ink placeholder:text-ink5 font-mono focus:border-accent-line focus:outline-none"
              />
            </div>
          </div>
          <button
            type="button"
            onClick={handleAddCustomDns}
            disabled={isAddingCustom || !customName.trim() || !customPrimary.trim()}
            className="flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold bg-hovert hover:bg-accent-fill hover:text-accent-contrast text-ink3 border border-line transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isAddingCustom ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Plus className="h-3.5 w-3.5" />
            )}
            <span>{t.dns.customAddBtn}</span>
          </button>
        </div>

        {/* Smart hosts */}
        <div className="rounded-panel border border-line bg-cardin p-4 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-violet-500/20 to-accent-fill/5 border border-accent-line text-violet-400 shadow-sm shadow-violet-500/5">
                <FileText className="h-4 w-4 shrink-0" />
              </div>
              <div>
                <h4 className="text-xs font-semibold text-ink">{t.smartHosts.title}</h4>
                <p className="text-[11px] text-ink4">{t.smartHosts.subtitle}</p>
              </div>
            </div>
            <span className={`rounded-chip px-1.5 py-0.5 text-[9px] font-semibold uppercase border shrink-0 ${
              hostsStatus?.enabled
                ? "bg-emerald-500/20 text-emerald-300 border-emerald-500/30"
                : "bg-hovert text-ink4 border-line"
            }`}>
              {hostsStatus?.enabled ? t.smartHosts.enabledBadge : t.smartHosts.disabledBadge}
            </span>
          </div>

          <p className="text-[11px] text-ink4 leading-relaxed">{t.smartHosts.howItWorks}</p>
          <p className="text-[10px] text-ink5 leading-relaxed">{t.smartHosts.backupNote}</p>

          {hostsError && (
            <div className="flex items-start gap-2.5 rounded-panel border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
              <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
              <span>{hostsError}</span>
            </div>
          )}

          {/* Sources */}
          <div className="space-y-1.5">
            <span className="text-[11px] font-medium text-ink3">{t.smartHosts.sourcesLabel}</span>
            {hostsStatus?.sources.map((s) => (
              <label
                key={s.id}
                className="flex items-start gap-2.5 rounded-control border border-line bg-card p-2.5 cursor-pointer hover:border-linestrong transition-colors"
              >
                <input
                  type="checkbox"
                  checked={s.enabled}
                  disabled={hostsBusy}
                  onChange={() => handleToggleSource(s.id)}
                  className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-emerald-500 cursor-pointer"
                />
                <span>
                  <span className="block text-xs font-semibold text-ink">{s.name}</span>
                  <span className="block text-[10px] text-ink4 leading-tight mt-0.5">{s.description}</span>
                </span>
              </label>
            ))}
          </div>

          {/* Adblock toggle */}
          <label className="flex items-start gap-2.5 rounded-control border border-line bg-card p-2.5 cursor-pointer hover:border-linestrong transition-colors">
            <input
              type="checkbox"
              checked={hostsStatus?.include_adblock ?? false}
              disabled={hostsBusy}
              onChange={handleToggleAdblock}
              className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-emerald-500 cursor-pointer"
            />
            <span>
              <span className="block text-xs font-semibold text-ink">{t.smartHosts.adblockLabel}</span>
              <span className="block text-[10px] text-ink4 leading-tight mt-0.5">{t.smartHosts.adblockDesc}</span>
            </span>
          </label>

          {/* Status line */}
          {hostsStatus?.enabled && (
            <div className="flex items-center gap-2 text-[11px] text-ink4">
              <Clock className="h-3.5 w-3.5 shrink-0" />
              <span>
                {t.smartHosts.lastUpdateLabel} {formatHostsDate(hostsStatus.last_update)}
                {hostsStatus.entry_count > 0 && (
                  <> · {hostsStatus.entry_count} {t.smartHosts.entriesLabel}</>
                )}
              </span>
            </div>
          )}

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={handleToggleSmartHosts}
              disabled={hostsBusy}
              className={`flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold border transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-50 ${
                hostsStatus?.enabled
                  ? "bg-hovert hover:bg-rose-500/15 text-ink3 hover:text-rose-300 border-line hover:border-rose-500/30"
                  : "bg-accent-fill hover:opacity-90 text-accent-contrast border-transparent"
              }`}
            >
              {hostsBusy && hostsBusyOp === "toggle" ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : null}
              <span>
                {hostsBusyOp === "toggle"
                  ? t.smartHosts.enabling
                  : hostsStatus?.enabled
                  ? t.smartHosts.disableBtn
                  : t.smartHosts.enableBtn}
              </span>
            </button>
            {hostsStatus?.enabled && (
              <button
                type="button"
                onClick={handleRefreshSmartHosts}
                disabled={hostsBusy}
                className="flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold bg-hovert hover:bg-accent-fill hover:text-accent-contrast text-ink3 border border-line transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${hostsBusyOp === "refresh" ? "animate-spin" : ""}`} />
                <span>{hostsBusyOp === "refresh" ? t.smartHosts.refreshing : t.smartHosts.refreshNowBtn}</span>
              </button>
            )}
          </div>
        </div>
      </div>

      {/* YouTube 4K Stream Speedtest Section */}
      <div className="t-card p-5 space-y-4 stagger-item" style={{ "--stagger-i": 1 } as React.CSSProperties}>
        <div className="flex items-center justify-between border-b border-line pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 min-w-[36px] max-w-[36px] aspect-square shrink-0 items-center justify-center rounded-panel bg-gradient-to-br from-red-500/20 to-red-600/5 border border-red-500/25 text-red-400 shadow-sm shadow-red-500/5">
              <Tv className="h-4 w-4 shrink-0" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-ink">{t.speedtest.title}</h3>
              <p className="text-[11px] text-ink4">{t.speedtest.subtitle}</p>
            </div>
          </div>
        </div>

        {speedError && (
          <div className="flex items-start gap-2.5 rounded-panel border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
            <span>{speedError}</span>
          </div>
        )}

        {/* Digital Benchmark Display */}
        <div className="rounded-panel border border-line bg-cardin p-5 flex flex-col items-center justify-center space-y-4">
          <div className="flex items-center gap-2 text-xs text-ink4">
            <Activity className="h-3.5 w-3.5 text-red-400" />
            <span>{t.speedtest.targetNode}</span>
          </div>

          {/* Speed Value & Live Pulse */}
          <div className="flex flex-col items-center justify-center py-2">
            {isSpeedRunning ? (
              <div className="flex flex-col items-center space-y-2">
                <Loader2 className="h-8 w-8 animate-spin text-red-400" />
                <span className="text-xs font-medium text-ink3">
                  {t.speedtest.measuringNote}
                </span>
                {/* Progress bar */}
                <div className="w-56 h-1.5 bg-hovert rounded-full overflow-hidden">
                  <div
                    className="h-full bg-red-500 transition-all duration-300 rounded-full"
                    style={{ width: `${speedProgress}%` }}
                  />
                </div>
              </div>
            ) : speedResult ? (
              <div className="flex flex-col items-center">
                <div className="flex items-baseline gap-1.5">
                  <span className="text-4xl font-extrabold tracking-tight text-ink font-mono">
                    {speedValue.toFixed(1)}
                  </span>
                  <span className="text-xs font-bold uppercase text-red-400 font-mono">
                    {t.speedtest.mbps}
                  </span>
                </div>
                {currentTier && (
                  <span
                    className={`mt-2 rounded-chip border px-2.5 py-0.5 text-xs font-semibold ${currentTier.badgeClass}`}
                  >
                    {currentTier.label}
                  </span>
                )}
              </div>
            ) : (
              <div className="flex flex-col items-center text-center py-2 space-y-1">
                <span className="text-2xl font-bold font-mono text-ink5">—</span>
                <span className="text-xs text-ink4">
                  {t.speedtest.pressToStart}
                </span>
              </div>
            )}
          </div>

          {/* Action Button */}
          <button
            type="button"
            onClick={handleStartSpeedtest}
            disabled={isSpeedRunning}
            className="flex items-center gap-2 rounded-panel bg-danger-fill hover:bg-danger-fill-hover px-6 py-2.5 text-xs font-semibold text-white shadow-sm transition-all active:scale-[0.98] cursor-pointer disabled:opacity-50"
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
          <div className="space-y-2 rounded-panel border border-line bg-cardin p-3.5">
            <div className="grid grid-cols-2 gap-3 text-xs">
              <div className="flex items-center gap-2">
                <Timer className="h-4 w-4 text-ink5" />
                <span className="text-ink4">{t.speedtest.resultDuration}</span>
                <span className="font-mono font-semibold text-ink2">
                  {((speedResult.duration_ms ?? speedResult.download_time_ms ?? 0) / 1000).toFixed(2)} {t.speedtest.secShort}
                </span>
              </div>
              <div className="flex items-center gap-2">
                <DownloadCloud className="h-4 w-4 text-ink5" />
                <span className="text-ink4">{t.speedtest.resultPayload}</span>
                <span className="font-mono font-semibold text-ink2">
                  {(speedResult.bytes_downloaded / (1024 * 1024)).toFixed(1)} {t.speedtest.mbShort}
                </span>
              </div>
            </div>
            {currentTier && (
              <div className="pt-2 border-t border-line text-[11px] text-ink4 flex items-center gap-1.5">
                <ShieldCheck className="h-3.5 w-3.5 text-emerald-400 shrink-0" />
                <span>{currentTier.desc}</span>
              </div>
            )}
          </div>
        )}

        {/* Speed Requirements Helper Card */}
        <div className="rounded-panel border border-line bg-cardin p-3 space-y-1.5 text-[11px]">
          <span className="font-semibold text-ink3">{t.speedtest.bitrateTitle}</span>
          <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-ink4">
            <div>• <span className="font-mono font-semibold">{t.speedtest.bitrate4k}</span></div>
            <div>• <span className="font-mono font-semibold">{t.speedtest.bitrate2k}</span></div>
            <div>• <span className="font-mono font-semibold">{t.speedtest.bitrateFhd}</span></div>
            <div>• <span className="font-mono font-semibold">{t.speedtest.bitrateHd}</span></div>
          </div>
        </div>
      </div>
      <DnsSplitModal isOpen={isDnsSplitOpen} onClose={() => setIsDnsSplitOpen(false)} />
    </div>
  );
});
