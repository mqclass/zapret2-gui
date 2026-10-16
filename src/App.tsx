import React, { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  SupervisorStatus,
  DiagnosticsReport,
  LuaStrategy,
  LogEntry,
  SystemStatusResponse,
  SupervisorStatusResponse,
  ConflictItem,
  SplitTunnelMode,
  AppSettings,
  ListSyncReport,
} from "./types";
import { Sidebar, NavTab } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { ConflictBanner } from "./components/ConflictBanner";
import { DashboardView } from "./components/views/DashboardView";
import { StrategiesView } from "./components/views/StrategiesView";
import { DiagnosticsView } from "./components/views/DiagnosticsView";
import { DnsSpeedView } from "./components/views/DnsSpeedView";
import { ListsView } from "./components/views/ListsView";
import { LogsView } from "./components/views/LogsView";
import { SettingsView } from "./components/views/SettingsView";
import { AlertCircle, CheckCircle2, ShieldAlert, X } from "lucide-react";
import { I18nProvider, useI18n } from "./i18n";
import { formatErrorMessage } from "./utils/error";

const AppContent: React.FC = () => {
  const { t } = useI18n();

  const [activeTab, setActiveTab] = useState<NavTab>("dashboard");
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
  const [splitTunnelMode, setSplitTunnelMode] = useState<SplitTunnelMode>("full");
  // Admin gate: shown when the app runs without Administrator privileges.
  const [showAdminGate, setShowAdminGate] = useState<boolean>(false);

  // Surface the admin gate once the real elevation state is known.
  useEffect(() => {
    if (!isElevated) {
      setShowAdminGate(true);
    }
  }, [isElevated]);

  const handleModeChange = useCallback(async (mode: SplitTunnelMode) => {
    try {
      const updated = await invoke<AppSettings>("set_split_tunnel_mode", { mode });
      setSplitTunnelMode(updated.split_tunnel_mode);
      if (status.state === "running") {
        setSuccessAlert(t.alerts.tunnelModeChanged);
      }
    } catch (err: any) {
      console.error("Failed to set split tunnel mode:", err);
      setErrorAlert(formatErrorMessage(err));
    }
  }, [status.state]);

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
      setErrorAlert(formatErrorMessage(err));
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

  // Monotonic client-side ids for stable React keys (backend entries have none).
  const logSeqRef = useRef(0);
  const assignLogIds = useCallback((entries: LogEntry[]): LogEntry[] => {
    return entries.map((e) =>
      e.id != null ? e : { ...e, id: logSeqRef.current++ }
    );
  }, []);

  // Flush pending log items to state every 200ms
  useEffect(() => {
    const timer = setInterval(() => {
      if (pendingLogsRef.current.length > 0) {
        const chunk = pendingLogsRef.current;
        pendingLogsRef.current = [];
        const withIds = assignLogIds(chunk);
        setLogs((prev) => {
          const next = [...prev, ...withIds];
          if (next.length > 1000) {
            return next.slice(next.length - 1000);
          }
          return next;
        });
      }
    }, 200);

    return () => clearInterval(timer);
  }, [assignLogIds]);

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

    // Load saved settings once and reuse the promise (Split-Tunneling / Game Mode)
    const settingsPromise = invoke<AppSettings>("get_app_settings");
    settingsPromise
      .then((cfg) => {
        if (cfg) {
          setSplitTunnelMode(cfg.split_tunnel_mode);
        }
      })
      .catch((err) => console.error("Failed to fetch app settings:", err));

    // 2. Discover strategies
    invoke<LuaStrategy[]>("list_strategies")
      .then((strats) => {
        setStrategies(strats);
        if (strats.length > 0) {
          settingsPromise
            .then((cfg) => {
              if (cfg?.last_strategy_id) {
                const matched = strats.find((s) => s.id === cfg.last_strategy_id);
                if (matched) {
                  setSelectedStrategyId(matched.id);
                  return;
                }
              }
              const defaultStrat = strats.find((s) => s.id === "flowseal-alt") || strats[0];
              if (defaultStrat) {
                setSelectedStrategyId(defaultStrat.id);
              }
            })
            .catch(() => {
              const defaultStrat = strats.find((s) => s.id === "flowseal-alt") || strats[0];
              if (defaultStrat) {
                setSelectedStrategyId(defaultStrat.id);
              }
            });
        }
      })
      .catch((err) => {
        console.error("Failed to discover Lua strategies:", err);
      });

    // 3. Initial logs query
    invoke<LogEntry[]>("get_logs")
      .then((initialLogs) => {
        setLogs(assignLogIds(initialLogs));
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

    // Lightweight heartbeat: supervisor state + elevation only (no process snapshot).
    // Conflict scanning is expensive (full process enumeration) — poll it rarely.
    const heartbeat = setInterval(() => {
      invoke<SupervisorStatusResponse>("get_supervisor_status")
        .then((res) => {
          setIsElevated(res.is_elevated);
          setStatus(res.supervisor);
        })
        .catch(() => {});
    }, 2500);

    const conflictsTimer = setInterval(() => {
      invoke<ConflictItem[]>("scan_conflicts")
        .then((res) => {
          setConflicts(res || []);
        })
        .catch(() => {});
    }, 60000);

    return () => {
      clearInterval(heartbeat);
      clearInterval(conflictsTimer);
      unlistenLogsPromise.then((unlisten) => unlisten());
      unlistenStatePromise.then((unlisten) => unlisten());
      unlistenProbesPromise.then((unlisten) => unlisten());
      unlistenClearPromise.then((unlisten) => unlisten());
    };
  }, [assignLogIds]);

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
      const msg = formatErrorMessage(err);
      setErrorAlert(msg);
      if (msg.includes("ADMIN_REQUIRED")) {
        setShowAdminGate(true);
      }
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
      const msg = formatErrorMessage(err);
      setErrorAlert(msg);
      if (msg.includes("ADMIN_REQUIRED")) {
        setShowAdminGate(true);
      }
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
      setErrorAlert(formatErrorMessage(err));
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    }
  }, [handleRefreshProbes]);

  const [isRefreshingLogs, setIsRefreshingLogs] = useState(false);

  // Manual log refresh (user-initiated): the live `log-batch` event stream is the
  // single continuous source — no competing poller (that caused lost/duplicated lines).
  const handleRefreshLogs = useCallback(async () => {
    setIsRefreshingLogs(true);
    try {
      const refreshed = await invoke<LogEntry[]>("get_logs");
      if (refreshed) {
        // Buffered entries are a subset of the ring buffer: drop them to avoid dupes.
        pendingLogsRef.current = [];
        setLogs(assignLogIds(refreshed));
      }
    } catch (err: any) {
      console.error("Failed to refresh logs:", err);
      setErrorAlert(formatErrorMessage(err));
    } finally {
      setIsRefreshingLogs(false);
    }
  }, [assignLogIds]);

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
      setErrorAlert(formatErrorMessage(err));
    }
  }, []);

  const handleKillStray = useCallback(async () => {
    setIsStrayCleaning(true);
    try {
      const count = await invoke<number>("kill_stray_processes");
      if (count > 0) {
        const entry: LogEntry = {
          id: logSeqRef.current++,
          timestamp: Math.floor(Date.now() / 1000),
          level: "warn",
          message: `Terminated ${count} orphaned winws2.exe process(es)`,
        };
        setSuccessAlert(`${t.alerts.strayTerminated} ${count}`);
        setLogs((prev) => [...prev, entry]);
      }
    } catch (err: any) {
      console.error("Failed to kill stray processes:", err);
      setErrorAlert(formatErrorMessage(err));
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
      const refreshed = await invoke<ConflictItem[]>("scan_conflicts");
      setConflicts(refreshed);
    } catch (err: any) {
      console.error("Failed to resolve conflicts:", err);
      setErrorAlert(formatErrorMessage(err));
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
      setErrorAlert(formatErrorMessage(err));
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
      setErrorAlert(formatErrorMessage(err));
    }
  }, [selectedStrategyId, t.alerts.configDeleted]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-[#0d1017] font-sans text-slate-100 select-none">
      {/* Sidebar Navigation */}
      <Sidebar
        activeTab={activeTab}
        onTabChange={setActiveTab}
        state={status.state}
        splitTunnelMode={splitTunnelMode}
        isElevated={isElevated}
      />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-w-0 h-full overflow-hidden bg-[#0d1017]">
        {/* Top Header Bar */}
        <div className="relative z-50">
          <TopBar
            activeTab={activeTab}
            isElevated={isElevated}
            onRelaunchElevated={handleRelaunchElevated}
            isAutostart={isAutostart}
            onToggleAutostart={handleToggleAutostart}
            isTogglingAutostart={isTogglingAutostart}
            onKillStray={handleKillStray}
            isStrayCleaning={isStrayCleaning}
          />
        </div>

        {/* Scrollable View Area */}
        <main className="relative z-10 flex-1 overflow-y-auto px-6 py-5 space-y-4">
          {/* Conflicting Software Warning Banner */}
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

          {/* Active View Router */}
          {activeTab === "dashboard" && (
            <DashboardView
              status={status}
              diagnostics={diagnostics}
              strategies={strategies}
              selectedStrategyId={selectedStrategyId}
              splitTunnelMode={splitTunnelMode}
              onModeChange={handleModeChange}
              onStart={handleStart}
              onStop={handleStop}
              onSelectStrategy={setSelectedStrategyId}
              onStartStrategy={handleStartStrategy}
              onRefreshProbes={handleRefreshProbes}
              isRefreshingProbes={isRefreshingProbes}
              onNavigateTab={setActiveTab}
            />
          )}

          {activeTab === "strategies" && (
            <StrategiesView
              strategies={strategies}
              selectedStrategyId={selectedStrategyId}
              activeStrategyId={status.active_strategy}
              supervisorState={status.state}
              onSelectStrategy={setSelectedStrategyId}
              onStartStrategy={handleStartStrategy}
              onStopStrategy={handleStop}
              onStart={handleStart}
              onSaveCustomProfile={handleSaveCustomProfile}
              onDeleteCustomProfile={handleDeleteCustomProfile}
              disabled={status.state === "starting" || status.state === "stopping"}
            />
          )}

          {activeTab === "diagnostics" && (
            <DiagnosticsView
              diagnostics={diagnostics}
              onRefreshProbes={handleRefreshProbes}
              isRefreshingProbes={isRefreshingProbes}
              onSelectAndStartStrategy={handleStartStrategy}
            />
          )}

          {activeTab === "dns" && <DnsSpeedView />}

          {activeTab === "lists" && <ListsView />}

          {activeTab === "logs" && (
            <LogsView
              logs={logs}
              onClearLogs={handleClearLogs}
              onRefreshLogs={handleRefreshLogs}
              isRefreshing={isRefreshingLogs}
            />
          )}

          {activeTab === "settings" && (
            <SettingsView
              isElevated={isElevated}
              onRelaunchElevated={handleRelaunchElevated}
              isAutostart={isAutostart}
              onToggleAutostart={handleToggleAutostart}
              isTogglingAutostart={isTogglingAutostart}
              onKillStray={handleKillStray}
              isStrayCleaning={isStrayCleaning}
              onResolveConflicts={handleResolveConflicts}
              isResolvingConflicts={isResolvingConflicts}
            />
          )}
        </main>
      </div>

      {/* Admin gate: shown when the app runs without Administrator privileges */}
      {showAdminGate && (
        <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
          <div className="w-full max-w-md rounded-2xl border border-amber-500/30 bg-[#141821] p-6 shadow-2xl">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-xl border border-amber-500/30 bg-amber-500/10">
                <ShieldAlert className="h-5 w-5 text-amber-400" />
              </div>
              <h2 className="text-base font-bold text-white">{t.adminGate.title}</h2>
            </div>
            <p className="mt-3 text-sm leading-relaxed text-slate-300">
              {t.adminGate.message}
            </p>
            <div className="mt-5 flex flex-col gap-2">
              <button
                onClick={handleRelaunchElevated}
                className="flex h-10 items-center justify-center gap-2 rounded-xl bg-amber-500/90 px-4 text-sm font-semibold text-black transition-colors hover:bg-amber-400 cursor-pointer"
              >
                <ShieldAlert className="h-4 w-4" />
                {t.adminGate.relaunch}
              </button>
              <button
                onClick={() => setShowAdminGate(false)}
                className="flex h-9 items-center justify-center rounded-xl border border-white/10 bg-white/[0.03] px-4 text-sm font-medium text-slate-300 transition-colors hover:bg-white/[0.07] cursor-pointer"
              >
                {t.adminGate.continueAnyway}
              </button>
            </div>
          </div>
        </div>
      )}
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
