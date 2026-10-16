import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe2,
  ShieldCheck,
  Check,
  Loader2,
  X,
  AlertCircle,
  Network,
  Zap,
} from "lucide-react";
import { DnsStatus } from "../types";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";

interface DnsManagerModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const DnsManagerModal: React.FC<DnsManagerModalProps> = ({
  isOpen,
  onClose,
}) => {
  const { t } = useI18n();
  const [dnsStatus, setDnsStatus] = useState<DnsStatus | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [applyingId, setApplyingId] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      loadDns();
    }
  }, [isOpen]);

  const loadDns = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const res = await invoke<DnsStatus>("get_dns_status");
      setDnsStatus(res);
    } catch (err: any) {
      console.error("Failed to load DNS status:", err);
      setError(formatErrorMessage(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleApplyDns = async (providerId: string) => {
    setApplyingId(providerId);
    setError(null);
    setMessage(null);
    try {
      const updated = await invoke<DnsStatus>("set_system_dns", {
        provider_id: providerId,
      });
      setDnsStatus(updated);
      setMessage(t.dns.successApplied);
      setTimeout(() => setMessage(null), 4000);
    } catch (err: any) {
      console.error("Failed to set system DNS:", err);
      setError(formatErrorMessage(err));
    } finally {
      setApplyingId(null);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div className="relative flex max-h-[90vh] w-full max-w-2xl flex-col rounded-2xl border border-slate-800 bg-slate-900 shadow-2xl">
        {/* Header */}
        <div className="flex items-start justify-between border-b border-slate-800 p-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-cyan-500 to-blue-600 shadow-md shadow-cyan-500/20">
              <Globe2 className="h-5 w-5 text-white" />
            </div>
            <div>
              <h2 className="text-base font-bold text-white">{t.dns.title}</h2>
              <p className="text-xs text-slate-400 mt-0.5">{t.dns.subtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-800 hover:text-white transition-colors cursor-pointer"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-5 space-y-4">
          {error && (
            <div className="flex items-start gap-3 rounded-xl border border-rose-500/30 bg-rose-500/10 p-4 text-xs text-rose-300">
              <AlertCircle className="h-4 w-4 shrink-0 text-rose-400 mt-0.5" />
              <div>
                <span className="font-semibold text-rose-200">Ошибка:</span>
                <p className="mt-0.5 leading-relaxed">{error}</p>
              </div>
            </div>
          )}

          {message && (
            <div className="flex items-center gap-2.5 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-3.5 text-xs text-emerald-300 font-medium">
              <ShieldCheck className="h-4 w-4 text-emerald-400" />
              <span>{message}</span>
            </div>
          )}

          {/* Active adapter info */}
          {dnsStatus && (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-800 bg-slate-950/70 p-4 text-xs">
              <div className="flex items-center gap-2.5">
                <Network className="h-4 w-4 text-cyan-400" />
                <div>
                  <span className="text-slate-400">{t.dns.adapterLabel}</span>
                  <div className="font-semibold text-white mt-0.5">
                    {dnsStatus.adapter_name}
                  </div>
                </div>
              </div>
              <div className="text-right">
                <span className="text-slate-400">{t.dns.currentDnsLabel}</span>
                <div className="font-mono text-cyan-300 font-medium mt-0.5">
                  {dnsStatus.current_dns.length > 0
                    ? dnsStatus.current_dns.join(", ")
                    : "DHCP (Автоматически)"}
                </div>
              </div>
            </div>
          )}

          {/* Providers List */}
          <div className="space-y-2.5">
            {isLoading ? (
              <div className="flex items-center justify-center p-8 text-slate-400 gap-2">
                <Loader2 className="h-5 w-5 animate-spin" />
                <span className="text-xs">Загрузка информации об адаптерах...</span>
              </div>
            ) : (
              dnsStatus?.available_providers.map((provider) => {
                const isActive = dnsStatus.active_provider_id === provider.id;
                const isApplying = applyingId === provider.id;

                return (
                  <div
                    key={provider.id}
                    className={`flex items-center justify-between gap-3 p-4 rounded-xl border transition-all ${
                      isActive
                        ? "border-cyan-500/50 bg-cyan-950/20 shadow-md shadow-cyan-950/30"
                        : "border-slate-800 bg-slate-950/50 hover:bg-slate-900/80 hover:border-slate-700/80"
                    }`}
                  >
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-bold text-xs text-white">
                          {provider.name}
                        </span>
                        {isActive && (
                          <span className="rounded-full bg-cyan-500/20 border border-cyan-500/40 px-2 py-0.5 text-[10px] font-bold text-cyan-300">
                            {t.dns.activeBadge}
                          </span>
                        )}
                        {provider.is_doh_capable && (
                          <span className="rounded-full bg-emerald-500/10 border border-emerald-500/30 px-2 py-0.5 text-[10px] font-semibold text-emerald-400">
                            {t.dns.dohBadge}
                          </span>
                        )}
                      </div>
                      <div className="font-mono text-[11px] text-slate-400">
                        {provider.primary ? `${provider.primary} / ${provider.secondary}` : "DHCP / Провайдер"}
                      </div>
                      <p className="text-[11px] text-slate-400 leading-relaxed">
                        {provider.description}
                      </p>
                    </div>

                    <div className="shrink-0">
                      {isActive ? (
                        <div className="flex items-center gap-1.5 text-xs font-semibold text-cyan-400 bg-cyan-500/10 border border-cyan-500/30 px-3 py-1.5 rounded-xl">
                          <Check className="h-3.5 w-3.5 stroke-[3]" />
                          <span>Применен</span>
                        </div>
                      ) : (
                        <button
                          onClick={() => handleApplyDns(provider.id)}
                          disabled={isApplying || applyingId !== null}
                          className="flex items-center gap-1.5 rounded-xl bg-slate-800 border border-slate-700 px-3 py-1.5 text-xs font-semibold text-slate-200 hover:bg-cyan-600 hover:border-cyan-500 hover:text-white transition-all cursor-pointer disabled:opacity-50"
                        >
                          {isApplying ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Zap className="h-3.5 w-3.5 text-cyan-400" />
                          )}
                          <span>{t.dns.applyBtn}</span>
                        </button>
                      )}
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end border-t border-slate-800 p-4">
          <button
            onClick={onClose}
            className="rounded-xl border border-slate-700 bg-slate-800 px-4 py-2 text-xs font-semibold text-slate-200 hover:bg-slate-700 transition-colors cursor-pointer"
          >
            {t.dns.closeBtn}
          </button>
        </div>
      </div>
    </div>
  );
};
