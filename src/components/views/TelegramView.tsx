import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Send,
  Play,
  Square,
  Loader2,
  Download,
  Copy,
  Check,
  RefreshCw,
  AlertTriangle,
  Info,
  ExternalLink,
  KeyRound,
} from "lucide-react";
import { TgProxyStatus, LogEntry, SupervisorState } from "../../types";
import { useI18n } from "../../i18n";
import { notifyUser } from "../../utils/notify";

interface TelegramViewProps {
  logs: LogEntry[];
}

type BusyKind = "idle" | "download" | "toggle" | "save" | "secret";

const SECRET_RE = /^[0-9a-fA-F]{32}$/;

const formatUptime = (secs: number, h: string, m: string, s: string) => {
  const hrs = Math.floor(secs / 3600);
  const mins = Math.floor((secs % 3600) / 60);
  const sec = secs % 60;
  if (hrs > 0) return `${hrs}${h} ${mins}${m} ${sec}${s}`;
  if (mins > 0) return `${mins}${m} ${sec}${s}`;
  return `${sec}${s}`;
};

export const TelegramView: React.FC<TelegramViewProps> = React.memo(({ logs }) => {
  const { t } = useI18n();
  const [status, setStatus] = useState<TgProxyStatus | null>(null);
  const [busy, setBusy] = useState<BusyKind>("idle");
  const [portInput, setPortInput] = useState("1443");
  const [secretInput, setSecretInput] = useState("");
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // T12: download progress (percent) streamed from the backend; null = idle/unknown.
  const [dlProgress, setDlProgress] = useState<number | null>(null);
  const copyTimer = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      const s = await invoke<TgProxyStatus>("get_tgproxy_status");
      setStatus(s);
      // T9: the 5s poll must not wipe a port the user is typing — same
      // activeElement guard as the secret field below.
      setPortInput((prev) => (document.activeElement?.tagName === "INPUT" ? prev : String(s.port)));
      setSecretInput((prev) => (document.activeElement?.tagName === "INPUT" ? prev : s.secret));
    } catch (e) {
      console.error("Failed to fetch tgproxy status:", e);
    }
  }, []);

  useEffect(() => {
    refresh();
    const unlistenPromise = listen<SupervisorState>("tgproxy-state", () => {
      refresh();
    });
    const timer = setInterval(refresh, 5000);
    return () => {
      clearInterval(timer);
      unlistenPromise.then((unlisten) => unlisten());
      if (copyTimer.current) window.clearTimeout(copyTimer.current);
    };
  }, [refresh]);

  useEffect(() => {
    const unlistenPromise = listen<number>("tgproxy-download-progress", (e) => {
      setDlProgress(e.payload);
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  const fail = useCallback(
    (e: unknown) => {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      notifyUser(t.telegram.notifyErrorTitle, msg).catch(() => {});
    },
    [t]
  );

  const handleDownload = useCallback(async () => {
    setBusy("download");
    setError(null);
    setDlProgress(0);
    try {
      const version = await invoke<string>("download_tgproxy");
      await notifyUser(t.telegram.notifyDownloadedTitle, `${t.telegram.notifyDownloadedBody}: ${version}`);
      await refresh();
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
      setDlProgress(null);
    }
  }, [refresh, fail, t]);

  const handleToggle = useCallback(async () => {
    if (!status || busy !== "idle") return;
    const running = status.state === "running";
    setBusy("toggle");
    setError(null);
    try {
      if (running) {
        await invoke("stop_tgproxy");
        await notifyUser(t.telegram.notifyStoppedTitle, t.telegram.notifyStoppedBody);
      } else {
        await invoke("start_tgproxy");
        await notifyUser(t.telegram.notifyStartedTitle, t.telegram.notifyStartedBody);
      }
      await refresh();
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  }, [status, busy, refresh, fail, t]);

  const handleGenerateSecret = useCallback(async () => {
    setBusy("secret");
    setError(null);
    try {
      const secret = await invoke<string>("regenerate_tgproxy_secret");
      setSecretInput(secret);
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  }, [fail]);

  const handleSave = useCallback(async () => {
    const port = Number(portInput);
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      setError(t.telegram.badPort);
      return;
    }
    if (!SECRET_RE.test(secretInput.trim())) {
      setError(t.telegram.badSecret);
      return;
    }
    setBusy("save");
    setError(null);
    try {
      await invoke("set_tgproxy_config", { port, secret: secretInput.trim().toLowerCase() });
      await notifyUser(t.telegram.notifyStoppedTitle, t.telegram.settingsSaved);
      await refresh();
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  }, [portInput, secretInput, refresh, fail, t]);

  const handleCopy = useCallback(async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(key);
      if (copyTimer.current) window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopiedKey(null), 1500);
    } catch {
      /* clipboard unavailable — ignore */
    }
  }, []);

  const handleOpenInTelegram = useCallback(async () => {
    if (!status?.tg_link) return;
    try {
      await openUrl(status.tg_link);
    } catch (e) {
      fail(e);
    }
  }, [status, fail]);

  const tgLogs = logs.filter((l) => l.level === "tgproxy").slice(-60);

  const state = status?.state ?? "stopped";
  const isRunning = state === "running";
  const isStarting = state === "starting";
  const isStopping = state === "stopping";
  const isTransitioning = isStarting || isStopping;
  const isError = state === "error";
  const hasSecret = !!status?.secret;

  const pillClass = isRunning
    ? "border-accent-line bg-okwash text-accent"
    : isError
    ? "border-danger-line bg-danger-wash text-ink"
    : isTransitioning
    ? "border-warnline bg-warnwash text-ink2"
    : "border-line bg-raise/60 text-ink4";

  const stateLabel = isRunning
    ? t.telegram.running
    : isStarting
    ? t.telegram.starting
    : isStopping
    ? t.telegram.stopping
    : isError
    ? t.telegram.error
    : t.telegram.stopped;

  return (
    <div className="space-y-4 pb-6">
      {/* Header / status card */}
      <div className="rounded-card border border-line bg-card p-6 shadow-sm">
        <div className="flex flex-col md:flex-row md:items-center justify-between gap-6">
          <div className="space-y-3">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-control bg-accent-wash text-accent border border-accent-line">
                <Send className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-lg font-semibold text-ink">{t.telegram.title}</h2>
                <p className="text-xs text-ink4">{t.telegram.subtitle}</p>
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <span
                className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-xs font-semibold tracking-wide uppercase ${pillClass}`}
              >
                <span
                  className={`h-2 w-2 rounded-full ${
                    isRunning
                      ? "bg-accent"
                      : isTransitioning
                      ? "bg-ink3 animate-pulse"
                      : isError
                      ? "bg-ink2"
                      : "bg-ink5"
                  }`}
                />
                {stateLabel}
              </span>
              {isRunning && status!.uptime_secs > 0 && (
                <span className="text-xs font-mono text-ink4">
                  {t.telegram.uptime}:{" "}
                  {formatUptime(
                    status!.uptime_secs,
                    t.telegram.uptimeH,
                    t.telegram.uptimeM,
                    t.telegram.uptimeS
                  )}
                </span>
              )}
              {isRunning && status!.pid && (
                <span className="text-xs font-mono text-ink5">
                  {t.telegram.pid}: {status!.pid}
                </span>
              )}
              {status?.binary_present && status.binary_version && (
                <span className="text-xs font-mono text-ink5">
                  {t.telegram.version}: {status.binary_version}
                </span>
              )}
            </div>
            {status?.binary_present && !status.version_supported && (
              <div className="flex items-start gap-2 rounded-panel border border-warnline bg-warnwash p-3 text-xs text-ink2">
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <span>{t.telegram.versionUntested}</span>
              </div>
            )}
            {(error || status?.last_error) && (
              <div className="flex items-start gap-2 rounded-panel border border-danger-line bg-danger-wash p-3 text-xs text-ink2">
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <span>
                  <span className="font-semibold">{t.telegram.lastError}: </span>
                  {error ?? status?.last_error}
                </span>
              </div>
            )}
          </div>

          <div className="flex flex-col gap-2 min-w-[220px]">
            {!status?.binary_present ? (
              <button
                onClick={handleDownload}
                disabled={busy === "download"}
                className="t-btn t-btn-primary"
              >
                {busy === "download" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Download className="h-4 w-4" />
                )}
                {busy === "download"
                  ? `${t.telegram.downloading}${dlProgress !== null ? ` ${dlProgress}%` : ""}`
                  : t.telegram.download}
              </button>
            ) : (
              <button
                onClick={handleToggle}
                disabled={busy !== "idle"}
                className={`t-btn ${isRunning ? "t-btn-danger" : "t-btn-primary"}`}
              >
                {busy === "toggle" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : isRunning ? (
                  <Square className="h-4 w-4" />
                ) : (
                  <Play className="h-4 w-4" />
                )}
                {isRunning ? t.telegram.disable : t.telegram.enable}
              </button>
            )}
            {!status?.binary_present && (
              <p className="text-[11px] leading-relaxed text-ink4">{t.telegram.notDownloadedDesc}</p>
            )}
          </div>
        </div>
      </div>

      {status?.binary_present && (
        <>
          {/* Connection parameters */}
          <div className="rounded-card border border-line bg-card p-6 shadow-sm space-y-4">
            <h3 className="text-sm font-semibold text-ink">{t.telegram.connectionTitle}</h3>
            <p className="text-xs text-ink4 leading-relaxed">{t.telegram.connectionHint}</p>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              {(
                [
                  { key: "host", label: t.telegram.host, value: status.host },
                  { key: "port", label: t.telegram.port, value: String(status.port) },
                  {
                    key: "secret",
                    label: t.telegram.secret,
                    value: status.secret || t.telegram.secretEmpty,
                    mono: true,
                    secret: true,
                  },
                ] as const
              ).map((f) => (
                <div key={f.key} className="rounded-panel border border-line bg-cardin p-3">
                  <div className="text-[11px] uppercase tracking-wide text-ink5 mb-1">{f.label}</div>
                  <div className="flex items-center justify-between gap-2">
                    <span
                      className={`text-sm truncate ${
                        "secret" in f && f.secret && !hasSecret
                          ? "text-ink5 italic"
                          : "font-mono text-ink"
                      }`}
                      title={f.value}
                    >
                      {f.value}
                    </span>
                    {hasSecret && (
                      <button
                        onClick={() => handleCopy(f.key, f.value)}
                        className="t-btn t-btn-ghost !px-2 !py-1"
                        title={t.telegram.copy}
                      >
                        {copiedKey === f.key ? (
                          <Check className="h-3.5 w-3.5 text-accent" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <button
                onClick={handleOpenInTelegram}
                disabled={!hasSecret}
                className="t-btn t-btn-accent-soft"
              >
                <ExternalLink className="h-4 w-4" />
                {t.telegram.openInTelegram}
              </button>
              {copiedKey === "tg_link" && (
                <span className="text-xs text-accent">{t.telegram.copied}</span>
              )}
              {hasSecret && (
                <button
                  onClick={() => handleCopy("tg_link", status.tg_link)}
                  className="t-btn t-btn-ghost"
                >
                  <Copy className="h-4 w-4" />
                  {t.telegram.tgLinkLabel}
                </button>
              )}
            </div>
          </div>

          {/* Settings */}
          <div className="rounded-card border border-line bg-card p-6 shadow-sm space-y-4">
            <h3 className="text-sm font-semibold text-ink">{t.telegram.settingsTitle}</h3>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <label className="space-y-1.5 block">
                <span className="text-xs text-ink4">{t.telegram.portLabel}</span>
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={portInput}
                  onChange={(e) => setPortInput(e.target.value)}
                  disabled={isRunning || busy !== "idle"}
                  className="t-input w-full font-mono"
                />
              </label>
              <div className="space-y-1.5">
                <span className="text-xs text-ink4">{t.telegram.secretLabel}</span>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={secretInput}
                    onChange={(e) => setSecretInput(e.target.value)}
                    disabled={isRunning || busy !== "idle"}
                    placeholder="—"
                    spellCheck={false}
                    className="t-input w-full font-mono"
                  />
                  <button
                    onClick={handleGenerateSecret}
                    disabled={isRunning || busy !== "idle"}
                    className="t-btn t-btn-ghost shrink-0"
                    title={t.telegram.generateSecret}
                  >
                    {busy === "secret" ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <KeyRound className="h-4 w-4" />
                    )}
                  </button>
                </div>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <button
                onClick={handleSave}
                disabled={isRunning || busy !== "idle"}
                className="t-btn t-btn-primary"
              >
                {busy === "save" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Check className="h-4 w-4" />
                )}
                {t.telegram.saveSettings}
              </button>
              {isRunning && (
                <span className="text-xs text-ink4">{t.telegram.mustStopFirst}</span>
              )}
            </div>
          </div>

          {/* Notes */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="flex items-start gap-2 rounded-card border border-accent-line bg-accent-wash p-4 text-xs text-ink2 leading-relaxed">
              <Info className="h-4 w-4 shrink-0 mt-0.5 text-accent" />
              <span>{t.telegram.parallelNote}</span>
            </div>
            <div className="flex items-start gap-2 rounded-card border border-line bg-cardin p-4 text-xs text-ink4 leading-relaxed">
              <Info className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{t.telegram.trayNote}</span>
            </div>
          </div>

          {/* Logs */}
          <div className="rounded-card border border-line bg-card p-6 shadow-sm space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-ink">{t.telegram.logsTitle}</h3>
              <button onClick={refresh} className="t-btn t-btn-ghost !px-2 !py-1" title={t.telegram.logsTitle}>
                <RefreshCw className="h-3.5 w-3.5" />
              </button>
            </div>
            <div className="rounded-panel border border-line bg-app p-3 h-56 overflow-y-auto font-mono text-[11px] leading-relaxed">
              {tgLogs.length === 0 ? (
                <span className="text-ink5">{t.telegram.noLogs}</span>
              ) : (
                tgLogs.map((l, i) => (
                  <div key={i} className="text-ink3 whitespace-pre-wrap break-all">
                    <span className="text-ink5">
                      {new Date(l.timestamp * 1000).toTimeString().split(" ")[0]}
                    </span>{" "}
                    {l.message}
                  </div>
                ))
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
});
