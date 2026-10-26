import React, { useState, useEffect } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  X,
  Plus,
  Trash2,
  Loader2,
  Globe,
  Lock,
} from "lucide-react";
import { useI18n } from "../i18n";

interface SplitDnsDomains {
  builtin: string[];
  custom: string[];
}

interface DnsSplitModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const DnsSplitModal: React.FC<DnsSplitModalProps> = ({ isOpen, onClose }) => {
  const { t } = useI18n();
  const [builtin, setBuiltin] = useState<string[]>([]);
  const [custom, setCustom] = useState<string[]>([]);
  const [newDomain, setNewDomain] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isAdding, setIsAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadDomains = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await invoke<SplitDnsDomains>("get_split_dns_domains");
      setBuiltin(result.builtin);
      setCustom(result.custom);
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      loadDomains();
    }
  }, [isOpen]);

  const handleAdd = async () => {
    if (!newDomain.trim()) return;
    setIsAdding(true);
    setError(null);
    try {
      const result = await invoke<SplitDnsDomains>("add_split_dns_domain", {
        domain: newDomain,
      });
      setBuiltin(result.builtin);
      setCustom(result.custom);
      setNewDomain("");
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsAdding(false);
    }
  };

  const handleRemove = async (domain: string) => {
    setError(null);
    try {
      const result = await invoke<SplitDnsDomains>("remove_split_dns_domain", {
        domain,
      });
      setBuiltin(result.builtin);
      setCustom(result.custom);
    } catch (err: any) {
      setError(typeof err === "string" ? err : JSON.stringify(err));
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      handleAdd();
    }
  };

  if (!isOpen) return null;

  // Portal to document.body: glass ancestors (backdrop-filter) would
  // otherwise become the containing block for `fixed` and confine the
  // backdrop to the card instead of the window (see CustomProfileModal).
  return createPortal(
    <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 modal-backdrop-enter">
      <div className="relative m-auto flex max-h-[90vh] w-full max-w-2xl flex-col rounded-card border border-raise bg-card shadow-2xl modal-panel-enter">
        {/* Header */}
        <div className="flex items-start justify-between border-b border-raise p-5">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-accent-wash border border-accent-line text-accent">
              <Globe className="h-5 w-5" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-ink">{t.dnsSplit.title}</h2>
              <p className="text-[11px] text-ink4">{t.dnsSplit.subtitle}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-control p-1.5 text-ink4 hover:text-ink hover:bg-hovert transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Add form */}
        <div className="border-b border-raise p-4">
          <div className="flex gap-2">
            <input
              type="text"
              value={newDomain}
              onChange={(e) => setNewDomain(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t.dnsSplit.placeholder}
              className="flex-1 rounded-control border border-line bg-cardin px-2.5 py-1.5 text-xs text-ink placeholder:text-ink5 font-mono focus:border-accent-line focus:outline-none"
            />
            <button
              onClick={handleAdd}
              disabled={isAdding || !newDomain.trim()}
              className="flex items-center gap-1.5 rounded-control px-3 py-1.5 text-xs font-semibold bg-hovert hover:bg-accent-fill hover:text-accent-contrast text-ink3 border border-line transition-all cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isAdding ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Plus className="h-3.5 w-3.5" />
              )}
              <span>{t.dnsSplit.add}</span>
            </button>
          </div>
          {error && <p className="mt-2 text-xs text-rose-400">{error}</p>}
        </div>

        {/* Domain lists */}
        <div className="flex-1 overflow-y-auto p-4 space-y-4 min-h-[200px]">
          {isLoading ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-5 w-5 animate-spin text-accent" />
            </div>
          ) : (
            <>
              {/* Custom domains */}
              {custom.length > 0 && (
                <div>
                  <h3 className="text-[11px] font-semibold text-ink4 uppercase mb-2">
                    {t.dnsSplit.customTitle} ({custom.length})
                  </h3>
                  <div className="space-y-1">
                    {custom.map((d) => (
                      <div
                        key={d}
                        className="flex items-center justify-between rounded-panel border border-line bg-cardin px-3 py-2"
                      >
                        <span className="text-xs text-ink font-mono">{d}</span>
                        <button
                          onClick={() => handleRemove(d)}
                          title={t.dnsSplit.remove}
                          className="p-1 text-ink5 hover:text-rose-400 hover:bg-rose-500/10 rounded-control transition-all cursor-pointer"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Built-in domains */}
              <div>
                <h3 className="text-[11px] font-semibold text-ink4 uppercase mb-2 flex items-center gap-1.5">
                  <Lock className="h-3 w-3" />
                  {t.dnsSplit.builtinTitle} ({builtin.length})
                </h3>
                <div className="flex flex-wrap gap-1.5">
                  {builtin.map((d) => (
                    <span
                      key={d}
                      className="rounded-chip border border-line bg-cardin px-2 py-0.5 text-[10px] text-ink4 font-mono"
                    >
                      {d}
                    </span>
                  ))}
                </div>
              </div>
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end border-t border-raise p-4">
          <button
            onClick={onClose}
            className="rounded-control px-4 py-1.5 text-xs font-semibold bg-accent-fill text-accent-contrast hover:opacity-90 transition-all cursor-pointer"
          >
            {t.dnsSplit.close}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
};
