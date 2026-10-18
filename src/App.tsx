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
  UpdateInfo,
  UpdateProgress,
  UpdatePrefs,
  SidecarStatusView,
} from "./types";
import { Sidebar, NavTab } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { ConflictBanner } from "./components/ConflictBanner";
import { UpdateDialog, UpdatePhase } from "./components/UpdateDialog";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { DashboardView } from "./components/views/DashboardView";
import { StrategiesView } from "./components/views/StrategiesView";
import { DiagnosticsView } from "./components/views/DiagnosticsView";
import { DnsSpeedView } from "./components/views/DnsSpeedView";
import { ListsView } from "./components/views/ListsView";
import { LogsView } from "./components/views/LogsView";
import { TelegramView } from "./components/views/TelegramView";
import { SettingsView } from "./components/views/SettingsView";
import { AlertCircle, CheckCircle2, ShieldAlert, X } from "lucide-react";
import { I18nProvider, useI18n } from "./i18n";
import { formatErrorMessage } from "./utils/error";
import { notifyUser, setNotificationsEnabled } from "./utils/notify";
import { save } from "@tauri-apps/plugin-dialog";

const AppContent: React.FC = () => {
  const { t } = useI18n();

  const [activeTab, setActiveTab] = useState<NavTab>("dashboard");
  const [isElevated, setIsElevated] = useState<boolean>(true);
  const [status, setStatus] = useState<SupervisorStatus>({
    state: "stopped",
    pid: null,
    active_strategy: null,
    active_transport: null,
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
  const [notificationsOn, setNotificationsOn] = useState<boolean>(true);
  const [isTogglingNotifications, setIsTogglingNotifications] = useState<boolean>(false);
  const [isExportingDiag, setIsExportingDiag] = useState<boolean>(false);
  const [splitTunnelMode, setSplitTunnelMode] = useState<SplitTunnelMode>("full");
  // Admin gate: shown when the app runs without Administrator privileges.
  const [showAdminGate, setShowAdminGate] = useState<boolean>(false);
  // Self-update state.
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [dismissedUpdateVersion, setDismissedUpdateVersion] = useState<string | null>(null);
  const [autoUpdateOn, setAutoUpdateOn] = useState<boolean>(true);
  const [isTogglingAutoUpdate, setIsTogglingAutoUpdate] = useState<boolean>(false);
  const [isCheckingUpdates, setIsCheckingUpdates] = useState<boolean>(false);
  const [updateDialogOpen, setUpdateDialogOpen] = useState<boolean>(false);
  const [updatePhase, setUpdatePhase] = useState<UpdatePhase>("confirm");
  const [updateProgress, setUpdateProgress] = useState<UpdateProgress | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  // Tracks the last version we already toasted about, so the 6h schedule
  // does not re-notify for the same release.
  const notifiedUpdateVersionRef = useRef<string | null>(null);

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

  const handleToggleNotifications = useCallback(async () => {
    setIsTogglingNotifications(true);
    try {
      const res = await invoke<boolean>("set_notifications_enabled", {
        enable: !notificationsOn,
      });
      setNotificationsOn(res);
      setNotificationsEnabled(res);
    } catch (err: any) {
      console.error("Failed to toggle notifications:", err);
      setErrorAlert(formatErrorMessage(err));
    } finally {
      setIsTogglingNotifications(false);
    }
  }, [notificationsOn]);

  // ---- Self-update ----------------------------------------------------

  /**
   * Runs a release check against GitHub.
   *
   * `manual=true` (the "Check for updates" button) surfaces errors and results
   * in the UI. `manual=false` (the 6h schedule) is quiet by design: a blocked
   * GitHub or missing network only logs to the console and retries on the
   * next tick — no error spam.
   */
  const runUpdateCheck = useCallback(
    async (manual: boolean) => {
      if (manual) setIsCheckingUpdates(true);
      try {
        const info = await invoke<UpdateInfo>("check_for_updates");
        setUpdateInfo(info);
        if (info.update_available) {
          // A newer release than the dismissed one re-arms the banner.
          if (notifiedUpdateVersionRef.current !== info.latest_version) {
            notifiedUpdateVersionRef.current = info.latest_version;
            notifyUser(
              t.update.notifyAvailableTitle,
              t.update.notifyAvailableBody.replace("{v}", info.latest_version)
            );
          }
          if (manual) setSuccessAlert(t.update.availableTitle.replace("{v}", info.latest_version));
        } else if (manual) {
          setSuccessAlert(t.update.upToDate);
        }
      } catch (err: any) {
        if (manual) {
          console.error("Update check failed:", err);
          setErrorAlert(`${t.update.checkFailed}: ${formatErrorMessage(err)}`);
        } else {
          // Quiet scheduled failure (GitHub blocked / offline): retry later.
          console.debug("Scheduled update check failed (will retry):", err);
        }
      } finally {
        if (manual) setIsCheckingUpdates(false);
      }
    },
    [t]
  );

  const handleCheckUpdates = useCallback(() => {
    setDismissedUpdateVersion(null);
    void runUpdateCheck(true);
  }, [runUpdateCheck]);

  const handleToggleAutoUpdate = useCallback(async () => {
    setIsTogglingAutoUpdate(true);
    try {
      const res = await invoke<boolean>("set_auto_update_enabled", {
        enable: !autoUpdateOn,
      });
      setAutoUpdateOn(res);
    } catch (err: any) {
      console.error("Failed to toggle auto-update:", err);
      setErrorAlert(formatErrorMessage(err));
    } finally {
      setIsTogglingAutoUpdate(false);
    }
  }, [autoUpdateOn]);

  const handleOpenUpdateDialog = useCallback(() => {
    setUpdatePhase("confirm");
    setUpdateProgress(null);
    setUpdateError(null);
    setUpdateDialogOpen(true);
  }, []);

  const handleDismissUpdate = useCallback(() => {
    setDismissedUpdateVersion(updateInfo?.latest_version ?? null);
  }, [updateInfo]);

  /**
   * Confirmed update flow: download (with progress + SHA256 verification,
   * cancellable), then stop the engine — only now, with the user's explicit
   * consent from the dialog and the verified payload on disk — then atomic
   * install and restart. On success the backend spawns the new version and
   * terminates this process, so this function never returns.
   */
  const handleConfirmUpdate = useCallback(async () => {
    if (!updateInfo) return;
    setUpdateError(null);
    setUpdateProgress(null);
    setUpdatePhase("downloading");

    let unlisten: (() => void) | undefined;
    let stage: "download" | "install" = "download";
    try {
      unlisten = await listen<UpdateProgress>("update-download-progress", (e) => {
        setUpdateProgress(e.payload);
      });

      await invoke<string>("download_update");

      // Fresh engine state (no stale React state): stop only if really running.
      const st = await invoke<SupervisorStatusResponse>("get_supervisor_status");
      if (st.supervisor.state === "running") {
        await invoke("stop_supervisor");
      }

      // Sidecars (e.g. the tgproxy sidecar once its branch is merged) also
      // execute from inside the installation directory: surface an active one
      // here with a clear message instead of hitting the backend's fail-closed
      // refusal mid-flow.
      const sidecars = await invoke<SidecarStatusView[]>("get_sidecar_statuses");
      const activeSidecars = sidecars.filter((s) => s.active);
      if (activeSidecars.length > 0) {
        throw new Error(
          `SIDECAR_RUNNING: ${activeSidecars.map((s) => s.name).join(", ")}`
        );
      }

      stage = "install";
      setUpdatePhase("installing");
      await invoke("install_update");

      // Reached only if the backend returned without installing or restarting.
      setUpdateError(t.update.installFailed);
      setUpdatePhase("error");
    } catch (err: any) {
      const raw = formatErrorMessage(err);
      // User-cancelled download: back to the confirmation step, no error.
      if (/CANCELLED/i.test(raw)) {
        setUpdateProgress(null);
        setUpdateError(null);
        setUpdatePhase("confirm");
        return;
      }
      let msg: string;
      if (/sha256|checksum/i.test(raw)) {
        msg = t.update.hashMismatch;
      } else if (/ENGINE_RUNNING/.test(raw)) {
        msg = t.update.engineRunning;
      } else if (/SIDECAR_RUNNING/.test(raw)) {
        const name = raw.split("SIDECAR_RUNNING:")[1]?.trim() || "?";
        msg = t.update.sidecarRunning.replace("{name}", name);
      } else if (/RELEASE_CHANGED/.test(raw)) {
        msg = t.update.releaseChanged;
      } else {
        msg =
          stage === "install"
            ? `${t.update.installFailed}: ${raw}`
            : `${t.update.downloadFailed}: ${raw}`;
      }
      console.error("Update failed:", err);
      setUpdateError(msg);
      setUpdatePhase("error");
    } finally {
      if (unlisten) unlisten();
    }
  }, [updateInfo, t]);

  /** Aborts an in-flight update download; the backend cleans up temp files. */
  const handleCancelUpdateDownload = useCallback(async () => {
    try {
      await invoke("cancel_update_download");
    } catch (err) {
      console.error("Failed to cancel update download:", err);
    }
  }, []);

  // Scheduled update checks: first run ~90s after startup (network settles),
  // then every 30 minutes — each tick only checks when the backend reports
  // the last *successful* check is older than 6h and the toggle is on.
  useEffect(() => {
    let cancelled = false;
    const UPDATE_CHECK_INTERVAL_MS = 6 * 3600 * 1000;

    const maybeScheduledCheck = async () => {
      try {
        const prefs = await invoke<UpdatePrefs>("get_update_prefs");
        if (cancelled) return;
        setAutoUpdateOn(prefs.auto_check_enabled);
        const stale =
          prefs.auto_check_enabled &&
          Date.now() / 1000 - prefs.last_check_epoch >= UPDATE_CHECK_INTERVAL_MS / 1000;
        if (stale) await runUpdateCheck(false);
      } catch (err) {
        if (!cancelled) console.debug("Scheduled update check skipped:", err);
      }
    };

    const timeoutId = window.setTimeout(maybeScheduledCheck, 90_000);
    const intervalId = window.setInterval(maybeScheduledCheck, 30 * 60_000);
    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
      window.clearInterval(intervalId);
    };
  }, [runUpdateCheck]);

  // Smart hosts: best-effort daily refresh of community hosts lists.
  // The backend only acts when 24h passed since the last refresh.
  useEffect(() => {
    let cancelled = false;
    const maybeRefreshHosts = async () => {
      try {
        await invoke("maybe_refresh_smart_hosts");
      } catch (err) {
        if (!cancelled) console.debug("Smart hosts refresh skipped:", err);
      }
    };
    const timeoutId = window.setTimeout(maybeRefreshHosts, 120_000);
    const intervalId = window.setInterval(maybeRefreshHosts, 6 * 3600_000);
    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
      window.clearInterval(intervalId);
    };
  }, []);

  const handleExportDiagnostics = useCallback(async () => {
    setIsExportingDiag(true);
    try {
      const stamp = new Date()
        .toISOString()
        .slice(0, 19)
        .replace(/[:T]/g, "-");
      const filePath = await save({
        defaultPath: `zapret2-diagnostics-${stamp}.txt`,
        filters: [{ name: "Text", extensions: ["txt"] }],
      });
      if (!filePath) return; // user cancelled the dialog
      await invoke<string>("export_diagnostics_bundle", { path: filePath });
      setSuccessAlert(t.settingsTab.exportDiagSuccess);
      notifyUser(t.settingsTab.exportDiagTitle, t.settingsTab.exportDiagSuccess);
    } catch (err: any) {
      console.error("Failed to export diagnostics:", err);
      setErrorAlert(`${t.settingsTab.exportDiagError}: ${formatErrorMessage(err)}`);
    } finally {
      setIsExportingDiag(false);
    }
  }, [t]);

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

  // Re-entrancy guard for engine start/stop: the backend start is slow
  // (svc attach up to ~10s) and the heartbeat must not re-enable the
  // buttons mid-flight (see heartbeat merge below). A ref (not state)
  // because handleStart's closure would read stale state.
  const engineTransitionRef = useRef(false);

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
          const notifEnabled = cfg.notifications_enabled ?? true;
          setNotificationsOn(notifEnabled);
          setNotificationsEnabled(notifEnabled);
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
          setStatus((prev) => {
            // Never let a slow poll clobber an in-flight start: while svc
            // attach is underway the backend still reports "stopped", which
            // would re-enable the start buttons mid-start. (Stopping has no
            // such slow window, so only "starting" is guarded.)
            if (prev.state === "starting" && res.supervisor.state === "stopped") {
              return prev;
            }
            return res.supervisor;
          });
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
    if (engineTransitionRef.current) return;
    engineTransitionRef.current = true;
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "starting" }));
    try {
      await invoke("start_supervisor", {
        strategy_id: selectedStrategyId || null,
        custom_args: customArgs?.length ? customArgs : null,
      });
      const stratName =
        strategies.find((s) => s.id === selectedStrategyId)?.name || selectedStrategyId || "";
      notifyUser(
        t.notify.engineStartedTitle,
        stratName ? `${t.notify.engineStartedBody}: ${stratName}` : t.notify.engineStartedBody
      );
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to start bypass:", err);
      const msg = formatErrorMessage(err);
      setErrorAlert(msg);
      notifyUser(t.notify.engineErrorTitle, msg);
      if (msg.includes("ADMIN_REQUIRED")) {
        setShowAdminGate(true);
      }
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    } finally {
      engineTransitionRef.current = false;
    }
  }, [selectedStrategyId, handleRefreshProbes, strategies, t]);

  const handleStartStrategy = useCallback(async (strategyId: string) => {
    if (engineTransitionRef.current) return;
    engineTransitionRef.current = true;
    setSelectedStrategyId(strategyId);
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "starting" }));
    try {
      await invoke("start_supervisor", {
        strategy_id: strategyId,
        custom_args: null,
      });
      const stratName = strategies.find((s) => s.id === strategyId)?.name || strategyId;
      notifyUser(
        t.notify.engineStartedTitle,
        `${t.notify.engineStartedBody}: ${stratName}`
      );
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to start strategy:", err);
      const msg = formatErrorMessage(err);
      setErrorAlert(msg);
      notifyUser(t.notify.engineErrorTitle, msg);
      if (msg.includes("ADMIN_REQUIRED")) {
        setShowAdminGate(true);
      }
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    } finally {
      engineTransitionRef.current = false;
    }
  }, [handleRefreshProbes, strategies, t]);

  const handleStop = useCallback(async () => {
    if (engineTransitionRef.current) return;
    engineTransitionRef.current = true;
    setErrorAlert(null);
    setSuccessAlert(null);
    setStatus((prev) => ({ ...prev, state: "stopping" }));
    try {
      await invoke("stop_supervisor");
      notifyUser(t.notify.engineStoppedTitle, t.notify.engineStoppedBody);
      setTimeout(() => {
        handleRefreshProbes();
      }, 1000);
    } catch (err: any) {
      console.error("Failed to stop bypass:", err);
      setErrorAlert(formatErrorMessage(err));
      setStatus((prev) => ({ ...prev, state: "stopped" }));
    } finally {
      engineTransitionRef.current = false;
    }
  }, [handleRefreshProbes, t]);

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
    <div className="app-bg flex h-screen w-screen overflow-hidden font-sans text-ink select-none gap-3 p-3">
      {/* Sidebar Navigation — floating frosted panel */}
      <Sidebar
        activeTab={activeTab}
        onTabChange={setActiveTab}
        state={status.state}
        splitTunnelMode={splitTunnelMode}
        isElevated={isElevated}
      />

      {/* Main Content Area — floating panel */}
      <div className="panel-float glass-bar flex-1 flex flex-col min-w-0 h-full overflow-hidden">
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
            onNavigateTab={setActiveTab}
          />
        </div>

        {/* Scrollable View Area */}
        <main key={activeTab} className="view-enter relative z-10 flex-1 overflow-y-auto px-6 py-5 space-y-4">
          <ErrorBoundary key={activeTab} label={activeTab}>
          {/* Conflicting Software Warning Banner */}
          <ConflictBanner
            conflicts={conflicts}
            onResolve={handleResolveConflicts}
            isResolving={isResolvingConflicts}
          />

          {/* Error Alert Banner */}
          {errorAlert && (
            <div className="flex items-start justify-between rounded-panel border border-rose-500/30 bg-rose-500/10 p-4 text-rose-300 backdrop-blur-md">
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
            <div className="flex items-start justify-between rounded-panel border border-emerald-500/30 bg-emerald-500/10 p-3.5 text-emerald-300 backdrop-blur-md">
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
              updateInfo={
                dismissedUpdateVersion && updateInfo?.latest_version === dismissedUpdateVersion
                  ? null
                  : updateInfo
              }
              onOpenUpdateDialog={handleOpenUpdateDialog}
              onDismissUpdate={handleDismissUpdate}
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

          {activeTab === "telegram" && <TelegramView logs={logs} />}

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
              notificationsOn={notificationsOn}
              onToggleNotifications={handleToggleNotifications}
              isTogglingNotifications={isTogglingNotifications}
              onExportDiagnostics={handleExportDiagnostics}
              isExportingDiag={isExportingDiag}
              autoUpdateOn={autoUpdateOn}
              onToggleAutoUpdate={handleToggleAutoUpdate}
              isTogglingAutoUpdate={isTogglingAutoUpdate}
              onCheckUpdates={handleCheckUpdates}
              isCheckingUpdates={isCheckingUpdates}
              updateInfo={updateInfo}
            />
          )}
          </ErrorBoundary>
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

      {/* Update confirmation / progress dialog */}
      {updateDialogOpen && updateInfo && (
        <UpdateDialog
          info={updateInfo}
          engineRunning={status.state === "running"}
          phase={updatePhase}
          progress={updateProgress}
          error={updateError}
          onConfirm={handleConfirmUpdate}
          onClose={() => setUpdateDialogOpen(false)}
          onCancelDownload={handleCancelUpdateDownload}
        />
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
