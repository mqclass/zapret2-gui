import React from "react";
import { ArrowDownToLine, X } from "lucide-react";
import { useI18n } from "../i18n";
import { UpdateInfo } from "../types";

interface UpdateBannerProps {
  info: UpdateInfo;
  onOpenDialog: () => void;
  onDismiss: () => void;
}

/**
 * Slim banner pinned to the top of the dashboard when a newer release is
 * available. It only *announces* — the download starts solely from the
 * confirmation dialog.
 */
export const UpdateBanner: React.FC<UpdateBannerProps> = ({
  info,
  onOpenDialog,
  onDismiss,
}) => {
  const { t } = useI18n();

  return (
    <div className="flex items-center justify-between gap-3 rounded-panel border border-accent-line bg-accent-wash p-3">
      <div className="flex items-center gap-2.5 min-w-0">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-control border border-accent-line bg-card">
          <ArrowDownToLine className="h-4 w-4 text-accent" />
        </div>
        <div className="min-w-0">
          <p className="truncate text-xs font-semibold text-ink">
            {t.update.availableTitle.replace("{v}", info.latest_version)}
          </p>
          <p className="truncate text-[11px] text-ink4">
            {t.update.currentVersionLabel}: v{info.current_version}
          </p>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <button type="button" onClick={onOpenDialog} className="t-btn t-btn-primary">
          {t.update.updateBtn}
        </button>
        <button
          type="button"
          onClick={onDismiss}
          className="rounded-control p-1.5 text-ink4 hover:bg-hovert hover:text-ink2 cursor-pointer"
          aria-label={t.update.laterBtn}
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
};
