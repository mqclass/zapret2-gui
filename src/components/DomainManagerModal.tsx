import React, { useState, useEffect, useMemo, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe,
  Plus,
  Trash2,
  ExternalLink,
  Search,
  RefreshCw,
  FolderOpen,
  X,
  CheckCircle2,
  AlertCircle,
  FileText,
  Sparkles,
} from "lucide-react";
import { useI18n } from "../i18n";

interface DomainManagerModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const DomainManagerModal: React.FC<DomainManagerModalProps> = ({
  isOpen,
  onClose,
}) => {
  const { t } = useI18n();

  const [files, setFiles] = useState<string[]>([]);
  const [selectedFile, setSelectedFile] = useState<string>("list-general.txt");
  const [domains, setDomains] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [newDomain, setNewDomain] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isFlushingDns, setIsFlushingDns] = useState(false);
  const [feedback, setFeedback] = useState<{ type: "success" | "error"; text: string } | null>(
    null
  );

  // Auto-dismiss feedback after 4 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [feedback]);

  // Load available files on modal open
  const loadFiles = useCallback(async () => {
    try {
      const fileList = await invoke<string[]>("list_domain_files");
      setFiles(fileList);
      if (fileList.length > 0 && !fileList.includes(selectedFile)) {
        // Prefer list-general.txt or list-general-user.txt
        const preferred =
          fileList.find((f) => f === "list-general.txt") ||
          fileList.find((f) => f === "list-general-user.txt") ||
          fileList[0];
        setSelectedFile(preferred);
      }
    } catch (e) {
      console.error("Failed to load domain files:", e);
    }
  }, [selectedFile]);

  // Load domains for current selected file
  const loadDomains = useCallback(async (file: string) => {
    if (!file) return;
    setIsLoading(true);
    try {
      const list = await invoke<string[]>("read_domain_list", { filename: file });
      setDomains(list);
    } catch (e) {
      setFeedback({ type: "error", text: String(e) });
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (isOpen) {
      loadFiles();
    }
  }, [isOpen, loadFiles]);

  useEffect(() => {
    if (isOpen && selectedFile) {
      loadDomains(selectedFile);
    }
  }, [isOpen, selectedFile, loadDomains]);

  // Filtered domains
  const filteredDomains = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return domains;
    return domains.filter((d) => d.toLowerCase().includes(q));
  }, [domains, searchQuery]);

  // Handle adding a new domain
  const handleAddDomain = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const raw = newDomain.trim();
    if (!raw) return;

    try {
      const updated = await invoke<string[]>("add_domain_to_list", {
        filename: selectedFile,
        domain: raw,
      });
      setDomains(updated);
      setNewDomain("");
      setFeedback({ type: "success", text: `${t.domainManager.domainAdded}: ${raw}` });
    } catch (err) {
      setFeedback({ type: "error", text: String(err) });
    }
  };

  // Handle removing a domain
  const handleRemoveDomain = async (domainToRemove: string) => {
    try {
      const updated = await invoke<string[]>("remove_domain_from_list", {
        filename: selectedFile,
        domain: domainToRemove,
      });
      setDomains(updated);
      setFeedback({ type: "success", text: `${t.domainManager.domainRemoved}: ${domainToRemove}` });
    } catch (err) {
      setFeedback({ type: "error", text: String(err) });
    }
  };

  // Handle flush DNS
  const handleFlushDns = async () => {
    setIsFlushingDns(true);
    try {
      const res = await invoke<string>("flush_dns_cache");
      setFeedback({ type: "success", text: res || t.domainManager.flushDnsSuccess });
    } catch (err) {
      setFeedback({ type: "error", text: String(err) });
    } finally {
      setIsFlushingDns(false);
    }
  };

  // Handle open lists folder
  const handleOpenFolder = async () => {
    try {
      await invoke("open_lists_folder");
    } catch (err) {
      setFeedback({ type: "error", text: String(err) });
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4 backdrop-blur-md animate-in fade-in duration-200">
      <div className="flex h-[90vh] max-h-[800px] w-full max-w-4xl flex-col rounded-2xl border border-slate-700/80 bg-slate-900 shadow-2xl shadow-black/80 overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/80 px-6 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gradient-to-tr from-emerald-500 to-teal-500 shadow-lg shadow-emerald-500/20 text-white">
              <Globe className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h2 className="text-lg font-bold text-white tracking-tight">
                  {t.domainManager.title}
                </h2>
                <span className="rounded-full bg-emerald-500/10 px-2.5 py-0.5 text-xs font-semibold text-emerald-400 border border-emerald-500/20">
                  {domains.length} {t.domainManager.totalDomains}
                </span>
              </div>
              <p className="text-xs text-slate-400">{t.domainManager.subtitle}</p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={handleFlushDns}
              disabled={isFlushingDns}
              className="flex items-center gap-1.5 rounded-lg border border-cyan-500/30 bg-cyan-500/10 px-3 py-1.5 text-xs font-medium text-cyan-300 hover:bg-cyan-500/20 transition-all disabled:opacity-50"
              title={t.domainManager.flushDnsBtn}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isFlushingDns ? "animate-spin" : ""}`} />
              {t.domainManager.flushDnsBtn}
            </button>

            <button
              onClick={handleOpenFolder}
              className="flex items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-800 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-700 hover:text-white transition-all"
              title={t.domainManager.openFolderBtn}
            >
              <FolderOpen className="h-3.5 w-3.5 text-slate-400" />
              {t.domainManager.openFolderBtn}
            </button>

            <button
              onClick={onClose}
              className="rounded-lg p-2 text-slate-400 hover:bg-slate-800 hover:text-white transition-all ml-2"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        {/* Feedback Alert Toast */}
        {feedback && (
          <div
            className={`flex items-center justify-between px-6 py-2.5 text-xs font-medium border-b animate-in slide-in-from-top-2 duration-150 ${
              feedback.type === "success"
                ? "bg-emerald-950/60 border-emerald-800/80 text-emerald-300"
                : "bg-red-950/60 border-red-800/80 text-red-300"
            }`}
          >
            <div className="flex items-center gap-2">
              {feedback.type === "success" ? (
                <CheckCircle2 className="h-4 w-4 text-emerald-400 shrink-0" />
              ) : (
                <AlertCircle className="h-4 w-4 text-red-400 shrink-0" />
              )}
              <span>{feedback.text}</span>
            </div>
            <button
              onClick={() => setFeedback(null)}
              className="text-slate-400 hover:text-white"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* List Selector Bar */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-800/80 bg-slate-900/90 px-6 py-3">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-slate-400">
              {t.domainManager.selectList}
            </span>
            <div className="flex items-center gap-1.5 overflow-x-auto max-w-lg scrollbar-thin">
              {files.map((f) => (
                <button
                  key={f}
                  onClick={() => setSelectedFile(f)}
                  className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-all ${
                    selectedFile === f
                      ? "bg-blue-600 text-white shadow-sm shadow-blue-500/20"
                      : "bg-slate-800/80 text-slate-300 hover:bg-slate-800 hover:text-white"
                  }`}
                >
                  <FileText className="h-3 w-3 opacity-70" />
                  {f}
                </button>
              ))}
            </div>
          </div>

          <div className="relative w-64">
            <Search className="absolute left-3 top-2.5 h-3.5 w-3.5 text-slate-500" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t.domainManager.searchPlaceholder}
              className="w-full rounded-lg border border-slate-700/80 bg-slate-950/70 py-1.5 pl-8 pr-3 text-xs text-white placeholder-slate-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>
        </div>

        {/* Add Domain Form */}
        <form
          onSubmit={handleAddDomain}
          className="flex items-center gap-3 border-b border-slate-800/80 bg-slate-950/40 px-6 py-3"
        >
          <div className="relative flex-1">
            <Globe className="absolute left-3 top-2.5 h-4 w-4 text-emerald-400" />
            <input
              type="text"
              value={newDomain}
              onChange={(e) => setNewDomain(e.target.value)}
              placeholder={t.domainManager.addDomainPlaceholder}
              className="w-full rounded-xl border border-slate-700 bg-slate-950/80 py-2 pl-9 pr-3 text-sm text-white placeholder-slate-500 focus:border-emerald-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
            />
          </div>
          <button
            type="submit"
            disabled={!newDomain.trim()}
            className="flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-emerald-600 to-teal-600 px-4 py-2 text-xs font-semibold text-white shadow-md shadow-emerald-600/20 hover:from-emerald-500 hover:to-teal-500 transition-all disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Plus className="h-4 w-4" />
            {t.domainManager.addBtn}
          </button>
        </form>

        {/* Domains List Body */}
        <div className="flex-1 overflow-y-auto px-6 py-4 space-y-1.5">
          {isLoading ? (
            <div className="flex h-40 items-center justify-center">
              <RefreshCw className="h-6 w-6 animate-spin text-slate-500" />
            </div>
          ) : filteredDomains.length === 0 ? (
            <div className="flex h-48 flex-col items-center justify-center text-center text-slate-500">
              <Globe className="h-10 w-10 stroke-[1.5] text-slate-600 mb-2" />
              <p className="text-sm font-medium">
                {searchQuery ? t.domainManager.noDomainsFound : t.domainManager.domainEmpty}
              </p>
            </div>
          ) : (
            filteredDomains.map((domain) => (
              <div
                key={domain}
                className="group flex items-center justify-between rounded-xl border border-slate-800/80 bg-slate-950/40 px-4 py-2.5 transition-all hover:border-slate-700 hover:bg-slate-800/40"
              >
                <div className="flex items-center gap-3">
                  <div className="h-2 w-2 rounded-full bg-emerald-500/80 ring-2 ring-emerald-500/20" />
                  <span className="font-mono text-sm text-slate-200 select-all">{domain}</span>
                </div>

                <div className="flex items-center gap-2 opacity-80 group-hover:opacity-100 transition-opacity">
                  <a
                    href={`https://${domain}`}
                    target="_blank"
                    rel="noreferrer"
                    className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-800 hover:text-emerald-400 transition-all"
                    title={t.domainManager.openInBrowser}
                  >
                    <ExternalLink className="h-4 w-4" />
                  </a>

                  <button
                    onClick={() => handleRemoveDomain(domain)}
                    className="rounded-lg p-1.5 text-slate-400 hover:bg-red-500/10 hover:text-red-400 transition-all"
                    title={t.domainManager.deleteDomain}
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </div>
            ))
          )}
        </div>

        {/* Footer Tip */}
        <div className="flex items-center justify-between border-t border-slate-800 bg-slate-950/80 px-6 py-3 text-xs text-slate-400">
          <div className="flex items-center gap-2">
            <Sparkles className="h-3.5 w-3.5 text-amber-400 shrink-0" />
            <span>{t.domainManager.tip}</span>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg border border-slate-700 bg-slate-800 px-3 py-1 text-xs font-medium text-slate-300 hover:bg-slate-700 hover:text-white transition-all"
          >
            {t.modal.closeBtn}
          </button>
        </div>
      </div>
    </div>
  );
};
