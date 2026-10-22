import React from "react";
import {
  ArrowDownToLine,
  X,
  ExternalLink,
  AlertTriangle,
  RefreshCw,
} from "lucide-react";
import { useI18n } from "../i18n";
import { UpdateInfo, UpdateProgress } from "../types";
import { openUrl } from "@tauri-apps/plugin-opener";

export type UpdatePhase = "confirm" | "downloading" | "installing" | "error";

interface UpdateDialogProps {
  info: UpdateInfo;
  engineRunning: boolean;
  phase: UpdatePhase;
  progress: UpdateProgress | null;
  error: string | null;
  onConfirm: () => void;
  onClose: () => void;
  /** Aborts an in-flight download (visible only during `downloading`). */
  onCancelDownload: () => void;
}

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(v >= 10 ? 0 : 1)} ${units[u]}`;
}

function formatDate(iso: string, locale: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/**
 * Update confirmation / progress dialog.
 *
 * Shown only after the user asked to see (or install) an available update —
 * nothing is ever downloaded before the explicit "Update" click.
 */
export const UpdateDialog: React.FC<UpdateDialogProps> = ({
  info,
  engineRunning,
  phase,
  progress,
  error,
  onConfirm,
  onClose,
  onCancelDownload,
}) => {
  const { t, language } = useI18n();
  const busy = phase === "downloading" || phase === "installing";

  const desc = t.update.availableDesc
    .replace("{date}", formatDate(info.published_at, language))
    .replace("{size}", formatBytes(info.asset_size));

  return (
    <div className="fixed inset-0 z-[210] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-card border border-line bg-card p-6 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-accent-line bg-accent-wash">
              <ArrowDownToLine className="h-5 w-5 text-accent" />
            </div>
            <div>
              <h2 className="text-base font-bold text-ink">{t.update.dialogTitle}</h2>
              <p className="text-xs text-ink4">
                {t.update.availableTitle.replace("{v}", info.latest_version)}
              </p>
            </div>
          </div>
          {!busy && (
            <button
              type="button"
              onClick={onClose}
              className="rounded-control p-1.5 text-ink4 hover:bg-hovert hover:text-ink2 cursor-pointer"
              aria-label={t.update.laterBtn}
            >
              <X className="h-4 w-4" />
            </button>
          )}
        </div>

        {/* Meta */}
        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-ink4">
          <span>
            {t.update.sizeLabel}:{" "}
            <span className="font-mono text-ink2">{formatBytes(info.asset_size)}</span>
          </span>
          <span>
            {t.update.publishedLabel}:{" "}
            <span className="text-ink2">{formatDate(info.published_at, language)}</span>
          </span>
          <button
            type="button"
            onClick={() => openUrl(info.release_url)}
            className="flex items-center gap-1 text-accent hover:text-accent-soft hover:underline cursor-pointer bg-transparent border-0 p-0"
          >
            <span>{t.update.viewRelease}</span>
            <ExternalLink className="h-3 w-3" />
          </button>
        </div>

        <p className="mt-1 text-[11px] text-ink5">{desc}</p>

        {/* Changelog */}
        <div className="mt-3">
          <h4 className="mb-1.5 text-xs font-semibold text-ink">{t.update.changelogTitle}</h4>
          <div className="max-h-56 overflow-y-auto rounded-panel border border-line bg-cardin p-3">
            <pre className="whitespace-pre-wrap break-words font-sans text-[11px] leading-relaxed text-ink3">
              {info.release_notes.trim() || t.update.notesEmpty}
            </pre>
          </div>
        </div>

        {/* Engine warning: the update must not kill an active bypass silently */}
        {engineRunning && phase === "confirm" && (
          <div className="mt-3 flex items-start gap-2 rounded-panel border border-warnline bg-warnwash p-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" />
            <p className="text-[11px] leading-relaxed text-ink3">{t.update.engineStopNote}</p>
          </div>
        )}

        {/* Progress */}
        {phase === "downloading" && (
          <div className="mt-4 space-y-2">
            <div className="flex items-center justify-between text-xs">
              <span className="flex items-center gap-2 text-ink2">
                <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
                {/* At 100% the backend is finalizing the SHA256 and extracting —
                    this is the integrity-verification stage. */}
                {(progress?.percent ?? 0) >= 100 ? t.update.verifying : t.update.downloading}
              </span>
              <span className="font-mono text-ink4">{progress?.percent ?? 0}%</span>
            </div>
            <div className="h-2 overflow-hidden rounded-full bg-hovert">
              <div
                className="h-full rounded-full bg-accent transition-all duration-200"
                style={{ width: `${progress?.percent ?? 0}%` }}
              />
            </div>
            {progress && progress.total_bytes > 0 && (
              <p className="text-[10px] text-ink5 font-mono">
                {formatBytes(progress.downloaded_bytes)} / {formatBytes(progress.total_bytes)}
              </p>
            )}
          </div>
        )}

        {phase === "installing" && (
          <div className="mt-4 flex items-center gap-2 text-xs text-ink2">
            <RefreshCw className="h-3.5 w-3.5 animate-spin text-accent" />
            {t.update.installing}
          </div>
        )}

        {/* Error */}
        {phase === "error" && error && (
          <div className="mt-3 flex items-start gap-2 rounded-panel border border-danger-line bg-danger-wash p-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-rose-400" />
            <p className="text-[11px] leading-relaxed text-ink3">{error}</p>
          </div>
        )}

        {/* Actions */}
        <div className="mt-5 flex justify-end gap-2">
          {!busy && phase !== "error" && (
            <button type="button" onClick={onClose} className="t-btn t-btn-ghost">
              {t.update.laterBtn}
            </button>
          )}
          {phase === "confirm" && (
            <button type="button" onClick={onConfirm} className="t-btn t-btn-primary">
              <ArrowDownToLine className="h-3.5 w-3.5" />
              {t.update.updateBtn}
            </button>
          )}
          {phase === "downloading" && (
            <button type="button" onClick={onCancelDownload} className="t-btn t-btn-ghost">
              {t.update.cancelBtn}
            </button>
          )}
          {phase === "error" && (
            <button type="button" onClick={onClose} className="t-btn t-btn-ghost">
              {t.update.laterBtn}
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
