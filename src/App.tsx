import React, { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  SupervisorStatus,
  DiagnosticsReport,
  LuaStrategy,
  LogEntry,
  SystemStatusResponse,
  ConflictItem,
} from "./types";
import { Header } from "./components/Header";
import { ConflictBanner } from "./components/ConflictBanner";
import { StatusBadge } from "./components/StatusBadge";
import { DiagnosticsCard } from "./components/DiagnosticsCard";
import { StrategySelector } from "./components/StrategySelector";
import { Controls } from "./components/Controls";
import { LogViewer } from "./components/LogViewer";
import { DomainManagerModal } from "./components/DomainManagerModal";
import { AlertCircle, CheckCircle2, X } from "lucide-react";
import { I18nProvider, useI18n } from "./i18n";

const AppContent: React.FC = () => {
  const { t } = useI18n();

  const [isElevated, setIsElevated] = useState<boolean>(true);
  const [status, setStatus] = useState<SupervisorStatus>({
    state: "stopped",
    pid: null,
    active_strategy: null,
    binary_path: null,
    uptime_secs: 0,
    total_logs_buffered: 0,
  });

  const [diagnostics, setDiagnostics] = useState<DiagnosticsReport>({
    youtube: {
      target: "YouTube / GoogleVideo",
      status: "checking",
      latency_ms: null,
      packet_loss_pct: 0,
      last_checked_epoch: 0,
      message: "Initializing probe...",
    },
    discord: {
      target: "Discord Voice / Gateway",
      status: "checking",
      latency_ms: null,
      packet_loss_pct: 0,
      last_checked_epoch: 0,
      message: "Initializing probe...",
    },
    timestamp: 0,
  });

  const [strategies, setStrategies] = useState<LuaStrategy[]>([]);
  const [selectedStrategyId, setSelectedStrategyId] = useState<string>("flowseal-alt");
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [conflicts, setConflicts] = useState<ConflictItem[]>([]);
  const [isResolvingConflicts, setIsResolvingConflicts] = useState<boolean>(false);
  const [isStrayCleaning, setIsStrayCleaning] = useState<boolean>(false);
  const [errorAlert, setErrorAlert] = useState<string | null>(null);
  const [successAlert, setSuccessAlert] = useState<string | null>(null);
  const [isRefreshingProbes, setIsRefreshingProbes] = useState<boolean>(false);
  const [isAutostart, setIsAutostart] = useState<boolean>(false);
  const [isTogglingAutostart, setIsTogglingAutostart] = useState<boolean>(false);
  const [isDomainManagerOpen, setIsDomainManagerOpen] = useState<boolean>(false);

  const handleToggleAutostart = useCallback(async () => {
    setIsTogglingAutostart(true);
    try {
      const nextState = !isAutostart;
      const res = await invoke<boolean>("set_autostart_status", { enable: nextState });
      setIsAutostart(res);
      setSuccessAlert(
        res ? t.header.autostartSuccessEnabled : t.header.autostartSuccessDisabled
      );
    } catch (err: any) {
      console.error("Failed to toggle autostart:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsTogglingAutostart(false);
    }
  }, [isAutostart, t]);

  const handleRefreshProbes = useCallback(async () => {
    setIsRefreshingProbes(true);
    try {
      const updated = await invoke<DiagnosticsReport>("refresh_probes");
      if (updated) setDiagnostics(updated);
    } catch (e) {
      console.error("Failed to refresh probes:", e);
    } finally {
      setIsRefreshingProbes(false);
    }
  }, []);

  // Buffer ref to throttle log updates and prevent high-frequency re-renders
  const pendingLogsRef = useRef<LogEntry[]>([]);

  // Flush pending log items to state every 200ms
  useEffect(() => {
    const timer = setInterval(() => {
      if (pendingLogsRef.current.length > 0) {
        const chunk = pendingLogsRef.current;
        pendingLogsRef.current = [];
        setLogs((prev) => {
          const next = [...prev, ...chunk];
          if (next.length > 1000) {
            return next.slice(next.length - 1000);
          }
          return next;
        });
      }
    }, 200);

    return () => clearInterval(timer);
  }, []);

  // Initial data loading and event subscriptions
  useEffect(() => {
    // 1. Initial full status query
    invoke<SystemStatusResponse>("get_system_status")
      .then((res) => {
        setIsElevated(res.is_elevated);
        setStatus(res.supervisor);
        setConflicts(res.conflicts || []);
        if (res.diagnostics) {
          setDiagnostics(res.diagnostics);
        }
        setIsAutostart(res.autostart_enabled);
      })
      .catch((err) => {
        console.error("Failed to query initial system status:", err);
      });

    // Also query probes immediately
    invoke<DiagnosticsReport>("get_probe_metrics")
      .then((diag) => {
        if (diag) setDiagnostics(diag);
      })
      .catch((err) => console.error("Failed to fetch initial probes:", err));

    // 2. Discover strategies
    invoke<LuaStrategy[]>("list_strategies")
      .then((strats) => {
        setStrategies(strats);
        if (strats.length > 0) {
          const defaultStrat = strats.find(
            (s) => s.id === "flowseal-general"
          ) || strats[0];
          if (defaultStrat) {
            setSelectedStrategyId(defaultStrat.id);
          }
        }
      })
      .catch((err) => {
        console.error("Failed to discover Lua strategies:", err);
      });

    // 3. Initial logs query
    invoke<LogEntry[]>("get_logs")
      .then((initialLogs) => {
        setLogs(initialLogs);
      })
      .catch((err) => console.error("Failed to fetch initial logs:", err));

    // 4. Listen for micro-batched logs from Rust supervisor and buffer them
    const unlistenLogsPromise = listen<LogEntry[]>("log-batch", (event) => {
      pendingLogsRef.current.push(...event.payload);
    });

    // 5. Listen for supervisor state transitions
    const unlistenStatePromise = listen<string>("state-changed", (event) => {
      setStatus((prev) => ({
        ...prev,
        state: event.payload as any,
      }));
    });

    // 6. Listen for real-time probe diagnostics updates
    const unlistenProbesPromise = listen<DiagnosticsReport>("probe-update", (event) => {
      setDiagnostics(event.payload);
    });

    // 7. Listen for logs-cleared event
    const unlistenClearPromise = listen<void>("logs-cleared", () => {
      pendingLogsRef.current = [];
      setLogs([]);
    });

    // Periodic heartbeat to refresh uptime, PID, and conflicts
    const heartbeat = setInterval(() => {
      invoke<SystemStatusResponse>("get_system_status")
        .then((res) => {
          setIsElevated(res.is_elevated);
          setStatus(res.supervisor);
          setConflicts(res.conflicts || []);
          if (res.diagnostics) {
            setDiagnostics(res.diagnostics);
          }
        })
        .catch(() => {});
    }, 2500);

    return () => {
      clearInterval(heartbeat);
      unlistenLogsPromise.then((unlisten) => unlisten());
      unlistenStatePromise.then((unlisten) => unlisten());
      unlistenProbesPromise.then((unlisten) => unlisten());
      unlistenClearPromise.then((unlisten) => unlisten());
    };
  }, []);

  // Auto-dismiss alerts after timeout
  useEffect(() => {
    if (successAlert) {
      const timer = setTimeout(() => setSuccessAlert(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [successAlert]);

  useEffect(() => {
    if (errorAlert) {
      const timer = setTimeout(() => setErrorAlert(null), 8000);
      return () => clearTimeout(timer);
    }
  }, [errorAlert]);

  const handleStart = useCallback(async (customArgs?: string[]) => {
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "starting" }));
    try {
      await invoke("start_supervisor", {
        strategyId: selectedStrategyId || null,
        customArgs: customArgs || null,
      });
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to start bypass:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    }
  }, [selectedStrategyId, handleRefreshProbes]);

  const handleStartStrategy = useCallback(async (strategyId: string) => {
    setSelectedStrategyId(strategyId);
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "starting" }));
    try {
      await invoke("start_supervisor", {
        strategyId: strategyId,
        customArgs: null,
      });
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to start strategy:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    }
  }, [handleRefreshProbes]);

  const handleStop = useCallback(async () => {
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "stopping" }));
    try {
      await invoke("stop_supervisor");
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to stop bypass:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    }
  }, [handleRefreshProbes]);

  const handleClearLogs = useCallback(async () => {
    try {
      await invoke("clear_logs");
      pendingLogsRef.current = [];
      setLogs([]);
    } catch (err: any) {
      console.error("Failed to clear logs:", err);
    }
  }, []);

  const handleRelaunchElevated = useCallback(async () => {
    try {
      await invoke("relaunch_elevated");
    } catch (err: any) {
      console.error("Failed to relaunch elevated:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
    }
  }, []);

  const handleKillStray = useCallback(async () => {
    setIsStrayCleaning(true);
    try {
      const count = await invoke<number>("kill_stray_processes");
      if (count > 0) {
        setSuccessAlert(`${t.alerts.strayTerminated} ${count}`);
        setLogs((prev) => [
          ...prev,
          {
            timestamp: Math.floor(Date.now() / 1000),
            level: "warn",
            message: `Terminated ${count} orphaned winws2.exe process(es)`,
          },
        ]);
      }
    } catch (err: any) {
      console.error("Failed to kill stray processes:", err);
    } finally {
      setIsStrayCleaning(false);
    }
  }, [t.alerts.strayTerminated]);

  const handleResolveConflicts = useCallback(async () => {
    setIsResolvingConflicts(true);
    setErrorAlert(null);
    try {
      const resolved = await invoke<number>("resolve_conflicts");
      setSuccessAlert(`${t.alerts.conflictsResolved} ${resolved}. ${t.conflicts.successMessage}`);
      // Re-scan conflicts
      const refreshed = await invoke<ConflictItem[]>("scan_conflicts");
      setConflicts(refreshed);
    } catch (err: any) {
      console.error("Failed to resolve conflicts:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsResolvingConflicts(false);
    }
  }, [t.alerts.conflictsResolved, t.conflicts.successMessage]);

  const handleSaveCustomProfile = useCallback(async (strategy: LuaStrategy) => {
    setErrorAlert(null);
    try {
      const updated = await invoke<LuaStrategy[]>("save_custom_profile", { strategy });
      setStrategies(updated);
      setSelectedStrategyId(strategy.id);
      setSuccessAlert(`${t.alerts.configSaved}: "${strategy.name}"`);
    } catch (err: any) {
      console.error("Failed to save custom profile:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
    }
  }, [t.alerts.configSaved]);

  const handleDeleteCustomProfile = useCallback(async (id: string) => {
    setErrorAlert(null);
    try {
      const updated = await invoke<LuaStrategy[]>("delete_custom_profile", { id });
      setStrategies(updated);
      if (selectedStrategyId === id) {
        const fallback = updated.find((s) => s.id === "flowseal-general") || updated[0];
        if (fallback) {
          setSelectedStrategyId(fallback.id);
        }
      }
      setSuccessAlert(t.alerts.configDeleted);
    } catch (err: any) {
      console.error("Failed to delete custom profile:", err);
      setErrorAlert(typeof err === "string" ? err : JSON.stringify(err));
    }
  }, [selectedStrategyId, t.alerts.configDeleted]);

  return (
    <div className="flex h-screen flex-col bg-slate-950 font-sans text-slate-100 select-none">
      {/* App Header with Language Switcher, Autostart and Privilege Status */}
      <Header
        isElevated={isElevated}
        onRelaunchElevated={handleRelaunchElevated}
        onKillStray={handleKillStray}
        isStrayCleaning={isStrayCleaning}
        isAutostart={isAutostart}
        onToggleAutostart={handleToggleAutostart}
        isTogglingAutostart={isTogglingAutostart}
        onOpenDomainManager={() => setIsDomainManagerOpen(true)}
      />

      {/* Main Content Area */}
      <main className="flex-1 overflow-y-auto px-6 py-5 space-y-4">
        {/* Conflicting Software Warning Banner (Flowseal / GoodbyeDPI / etc.) */}
        <ConflictBanner
          conflicts={conflicts}
          onResolve={handleResolveConflicts}
          isResolving={isResolvingConflicts}
        />

        {/* Error Alert Banner */}
        {errorAlert && (
          <div className="flex items-start justify-between rounded-xl border border-rose-500/30 bg-rose-500/10 p-4 text-rose-300 backdrop-blur-md">
            <div className="flex items-start gap-2.5">
              <AlertCircle className="h-5 w-5 shrink-0 text-rose-400 mt-0.5" />
              <div className="text-xs">
                <div className="font-semibold text-rose-200">{t.alerts.faultTitle}</div>
                <div className="mt-0.5 leading-relaxed">{errorAlert}</div>
              </div>
            </div>
            <button
              onClick={() => setErrorAlert(null)}
              className="text-rose-400 hover:text-rose-200 transition-colors cursor-pointer"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        )}

        {/* Success Alert Banner */}
        {successAlert && (
          <div className="flex items-start justify-between rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-3.5 text-emerald-300 backdrop-blur-md">
            <div className="flex items-center gap-2.5 text-xs">
              <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-400" />
              <span className="font-medium">{successAlert}</span>
            </div>
            <button
              onClick={() => setSuccessAlert(null)}
              className="text-emerald-400 hover:text-emerald-200 transition-colors cursor-pointer"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        )}

        {/* Live Status Header Banner */}
        <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-slate-800 bg-slate-900/60 p-4 backdrop-blur-md">
          <div className="flex flex-col gap-1.5">
            <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
              {t.status.title}
            </span>
            <StatusBadge
              state={status.state}
              pid={status.pid}
              activeStrategy={status.active_strategy}
              uptimeSecs={status.uptime_secs}
            />
          </div>

          <div className="text-right text-xs text-slate-400">
            {status.binary_path && (
              <div
                className="truncate max-w-md font-mono text-[11px] text-slate-400"
                title={status.binary_path}
              >
                {t.status.binary}: {status.binary_path}
              </div>
            )}
          </div>
        </div>

        {/* Real-Time Network Probes (YouTube & Discord) */}
        <DiagnosticsCard
          report={diagnostics}
          onRefresh={handleRefreshProbes}
          isRefreshing={isRefreshingProbes}
          onSelectAndStartStrategy={handleStartStrategy}
        />

        {/* Strategy Selection */}
        {(() => {
          const activeStratObj = strategies.find((s) => s.id === status.active_strategy);
          const selectedStratObj = strategies.find((s) => s.id === selectedStrategyId);
          const activeStratName = activeStratObj?.name || status.active_strategy;
          const selectedStratName = selectedStratObj?.name || selectedStrategyId;

          return (
            <>
              <StrategySelector
                strategies={strategies}
                selectedStrategyId={selectedStrategyId}
                activeStrategyId={status.active_strategy}
                supervisorState={status.state}
                onSelectStrategy={setSelectedStrategyId}
                onStartStrategy={handleStartStrategy}
                onStopStrategy={handleStop}
                onSaveCustomProfile={handleSaveCustomProfile}
                onDeleteCustomProfile={handleDeleteCustomProfile}
                disabled={status.state === "starting" || status.state === "stopping"}
              />

              {/* Primary Controls (Start / Stop / Advanced args) */}
              <Controls
                state={status.state}
                activeStrategyName={activeStratName}
                selectedStrategyName={selectedStratName}
                onStart={handleStart}
                onStop={handleStop}
                onSwitchToSelected={() => handleStartStrategy(selectedStrategyId)}
                disabled={false}
              />
            </>
          );
        })()}

        {/* Real-time Log Stream Viewer (Throttled & capped at 100 in DOM) */}
        <LogViewer logs={logs} onClearLogs={handleClearLogs} />
      </main>

      {/* Domain and Hostlist Manager Modal */}
      <DomainManagerModal
        isOpen={isDomainManagerOpen}
        onClose={() => setIsDomainManagerOpen(false)}
      />
    </div>
  );
};

export const App: React.FC = () => {
  return (
    <I18nProvider>
      <AppContent />
    </I18nProvider>
  );
};

export default App;
