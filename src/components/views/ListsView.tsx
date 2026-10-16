import React, { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Globe,
  Plus,
  Trash2,
  Search,
  RefreshCw,
  FolderOpen,
  CheckCircle2,
  AlertCircle,
  FileText,
  Sparkles,
} from "lucide-react";
import { useI18n } from "../../i18n";
import { ListSyncReport } from "../../types";
import { notifyUser } from "../../utils/notify";
import { formatErrorMessage } from "../../utils/error";

export const ListsView: React.FC = React.memo(() => {
  const { t } = useI18n();

  const [files, setFiles] = useState<string[]>([]);
  const [selectedFile, setSelectedFile] = useState<string>(() => {
    try {
      return localStorage.getItem("zapret2_selected_host_list") || "list-general.txt";
    } catch {
      return "list-general.txt";
    }
  });
  const [domains, setDomains] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [newDomain, setNewDomain] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isFlushingDns, setIsFlushingDns] = useState(false);
  const [isSyncing, setIsSyncing] = useState(false);
  const [feedback, setFeedback] = useState<{ type: "success" | "error"; text: string } | null>(
    null
  );

  const changeSelectedFile = useCallback((file: string) => {
    setSelectedFile(file);
    try {
      localStorage.setItem("zapret2_selected_host_list", file);
    } catch {
      // Ignore storage errors
    }
  }, []);

  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [feedback]);

  // Mirror of the selected file for the stable loadFiles callback below.
  const selectedFileRef = useRef(selectedFile);
  selectedFileRef.current = selectedFile;

  const loadFiles = useCallback(async () => {
    try {
      const fileList = await invoke<string[]>("list_domain_files");
      setFiles(fileList);
      if (fileList.length > 0 && !fileList.includes(selectedFileRef.current)) {
        const saved = localStorage.getItem("zapret2_selected_host_list");
        const preferred =
          (saved && fileList.includes(saved) ? saved : null) ||
          fileList.find((f) => f === "list-general.txt") ||
          fileList.find((f) => f === "list-general-user.txt") ||
          fileList[0];
        setSelectedFile(preferred);
      }
    } catch (e) {
      setFeedback({ type: "error", text: `Не удалось загрузить список файлов: ${String(e)}` });
    }
  }, []);

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
    loadFiles();
  }, [loadFiles]);

  useEffect(() => {
    if (selectedFile) {
      loadDomains(selectedFile);
    }
  }, [selectedFile, loadDomains]);

  const filteredDomains = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return domains;
    return domains.filter((d) => d.toLowerCase().includes(q));
  }, [domains, searchQuery]);

  const handleAddDomain = async (e: React.FormEvent) => {
    e.preventDefault();
    const domainToAdd = newDomain.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "");
    if (!domainToAdd) return;

    try {
      await invoke("add_domain_to_list", {
        filename: selectedFile,
        domain: domainToAdd,
      });
      setNewDomain("");
      setFeedback({ type: "success", text: `${t.domainManager.domainAdded}: ${domainToAdd}` });
      await loadDomains(selectedFile);
    } catch (err) {
      setFeedback({ type: "error", text: formatErrorMessage(err) });
    }
  };

  const handleRemoveDomain = async (domainToRemove: string) => {
    try {
      await invoke("remove_domain_from_list", {
        filename: selectedFile,
        domain: domainToRemove,
      });
      setFeedback({ type: "success", text: `${t.domainManager.domainRemoved}: ${domainToRemove}` });
      await loadDomains(selectedFile);
    } catch (err) {
      setFeedback({ type: "error", text: formatErrorMessage(err) });
    }
  };

  const handleFlushDns = async () => {
    setIsFlushingDns(true);
    try {
      await invoke("flush_dns_cache");
      setFeedback({ type: "success", text: t.domainManager.flushDnsSuccess });
    } catch (err) {
      setFeedback({ type: "error", text: formatErrorMessage(err) });
    } finally {
      setIsFlushingDns(false);
    }
  };

  const handleSyncRemoteLists = async () => {
    setIsSyncing(true);
    try {
      const report = await invoke<ListSyncReport>("sync_remote_lists");
      const updatedCount = report.files_updated ?? report.files?.length ?? 0;
      setFeedback({
        type: "success",
        text: `${t.syncLists.successTitle}: ${updatedCount} ${t.syncLists.filesUpdated}`,
      });
      notifyUser(
        t.notify.listsUpdatedTitle,
        `${t.notify.listsUpdatedBody}: ${updatedCount} ${t.syncLists.filesUpdated}`
      );
      await loadFiles();
      if (selectedFile) await loadDomains(selectedFile);
    } catch (err) {
      setFeedback({ type: "error", text: formatErrorMessage(err) });
    } finally {
      setIsSyncing(false);
    }
  };

  const handleOpenFolder = async () => {
    try {
      await invoke("open_lists_folder");
    } catch (err) {
      setFeedback({ type: "error", text: formatErrorMessage(err) });
    }
  };

  return (
    <div className="space-y-6 pb-6">
      {/* Header and Sync Banner */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 rounded-card border border-white/[0.08] bg-card/60 p-5 backdrop-blur-xl">
        <div>
          <h3 className="text-sm font-bold text-ink flex items-center gap-2">
            <span>{t.domainManager.title}</span>
            <span className="rounded-chip bg-accent-wash border border-accent-line px-2 py-0.5 text-[10px] font-mono text-accent-soft">
              {domains.length} {t.domainManager.totalDomains}
            </span>
          </h3>
          <p className="text-xs text-ink4 mt-1">
            {t.domainManager.subtitle}
          </p>
        </div>

        <div className="flex items-center gap-2.5">
          {/* GitHub Sync Button */}
          <button
            onClick={handleSyncRemoteLists}
            disabled={isSyncing}
            className="flex items-center gap-1.5 rounded-panel border border-accent-line bg-accent-wash hover:bg-accent-wash px-3.5 py-2 text-xs font-semibold text-accent-soft transition-all cursor-pointer disabled:opacity-50"
            title={t.syncLists.syncBtnTitle}
          >
            {isSyncing ? (
              <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
            ) : (
              <Sparkles className="h-3.5 w-3.5 text-accent" />
            )}
            <span>{isSyncing ? t.syncLists.syncing : t.syncLists.syncBtn}</span>
          </button>

          {/* Flush DNS Button */}
          <button
            onClick={handleFlushDns}
            disabled={isFlushingDns}
            className="flex items-center gap-1.5 rounded-panel border border-white/[0.08] bg-white/[0.03] hover:bg-white/[0.08] px-3.5 py-2 text-xs font-semibold text-ink3 transition-all cursor-pointer disabled:opacity-50"
            title={t.syncLists.flushDnsTitle}
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isFlushingDns ? "animate-spin" : ""}`} />
            <span>{t.domainManager.flushDnsBtn}</span>
          </button>

          {/* Open Folder in Explorer */}
          <button
            onClick={handleOpenFolder}
            className="p-2 rounded-panel border border-white/[0.08] bg-white/[0.03] hover:bg-white/[0.08] text-ink4 hover:text-ink transition-all cursor-pointer"
            title={t.domainManager.openFolderBtn}
          >
            <FolderOpen className="h-4 w-4" />
          </button>
        </div>
      </div>

      {feedback && (
        <div
          className={`flex items-center gap-2.5 rounded-panel border p-3 text-xs ${
            feedback.type === "success"
              ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
              : "border-rose-500/30 bg-rose-500/10 text-rose-300"
          }`}
        >
          {feedback.type === "success" ? (
            <CheckCircle2 className="h-4 w-4 text-emerald-400 shrink-0" />
          ) : (
            <AlertCircle className="h-4 w-4 text-rose-400 shrink-0" />
          )}
          <span>{feedback.text}</span>
        </div>
      )}

      {/* Main Container with File Tabs + Domain List */}
      <div className="rounded-card border border-white/[0.08] bg-card/60 overflow-hidden backdrop-blur-xl">
        {/* File Tabs */}
        <div className="flex items-center gap-2 overflow-x-auto border-b border-white/[0.06] bg-cardin/40 p-3">
          {files.map((file) => {
            const isSelected = selectedFile === file;
            return (
              <button
                key={file}
                onClick={() => changeSelectedFile(file)}
                className={`flex items-center gap-2 rounded-control px-3 py-1.5 text-xs font-semibold transition-all cursor-pointer shrink-0 ${
                  isSelected
                    ? "bg-accent-fill text-ink shadow-md shadow-accent/20"
                    : "text-ink4 hover:bg-white/[0.04] hover:text-ink2"
                }`}
              >
                <FileText className="h-3.5 w-3.5" />
                <span>{file}</span>
              </button>
            );
          })}
        </div>

        {/* Search & Add Domain Bar */}
        <div className="p-4 border-b border-white/[0.06] grid grid-cols-1 md:grid-cols-2 gap-3 bg-cardin/20">
          {/* Search Box */}
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-ink5" />
            <input
              type="text"
              placeholder={t.domainManager.searchPlaceholder}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full rounded-panel border border-white/[0.08] bg-cardin/60 pl-9 pr-3 py-2 text-xs text-ink placeholder-ink5 focus:border-accent-line focus:outline-none"
            />
          </div>

          {/* Add Domain Form */}
          <form onSubmit={handleAddDomain} className="flex gap-2">
            <input
              type="text"
              placeholder={t.domainManager.addDomainPlaceholder}
              value={newDomain}
              onChange={(e) => setNewDomain(e.target.value)}
              className="flex-1 rounded-panel border border-white/[0.08] bg-cardin/60 px-3 py-2 text-xs text-ink placeholder-ink5 focus:border-accent-line focus:outline-none"
            />
            <button
              type="submit"
              disabled={!newDomain.trim()}
              className="flex items-center gap-1.5 rounded-panel bg-accent-fill hover:bg-accent text-ink px-4 py-2 text-xs font-semibold shadow-md shadow-accent/20 transition-all cursor-pointer disabled:opacity-40"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>{t.domainManager.addBtn}</span>
            </button>
          </form>
        </div>

        {/* Domain List Body */}
        <div className="max-h-[460px] min-h-[240px] overflow-y-auto p-4">
          {isLoading ? (
            <div className="flex h-48 flex-col items-center justify-center gap-2 text-ink4">
              <RefreshCw className="h-6 w-6 animate-spin text-accent" />
              <span className="text-xs">Загрузка доменов...</span>
            </div>
          ) : filteredDomains.length === 0 ? (
            <div className="flex h-48 flex-col items-center justify-center gap-2 text-ink5">
              <Globe className="h-8 w-8 text-ink5" />
              <span className="text-xs">{t.domainManager.noDomainsFound}</span>
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-2">
              {filteredDomains.map((domain) => (
                <div
                  key={domain}
                  className="flex items-center justify-between rounded-control border border-white/[0.04] bg-cardin/40 px-3 py-2 hover:border-white/[0.1] hover:bg-cardin/70 transition-all group"
                >
                  <span className="truncate text-xs text-ink3 font-mono" title={domain}>
                    {domain}
                  </span>
                  <button
                    onClick={() => handleRemoveDomain(domain)}
                    className="p-1 rounded-chip text-ink5 hover:text-rose-400 hover:bg-rose-500/10 transition-colors opacity-0 group-hover:opacity-100 cursor-pointer"
                    title={t.domainManager.deleteDomain}
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
});
