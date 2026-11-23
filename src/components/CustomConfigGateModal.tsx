import React from "react";
import { createPortal } from "react-dom";
import { Settings2, X } from "lucide-react";
import { useI18n } from "../i18n";

interface CustomConfigGateModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Switch the shell to expert mode (the host owns the state). */
  onGoExpert: () => void;
}

/**
 * Simple-view gate for the «Свой конфиг» goal: the custom-profile
 * modal is fully expert (engine picker, raw launch args), so the
 * simple wizard shows this short interstitial instead — plain words
 * and an explicit way into the expert view. The expert view opens
 * CustomProfileModal directly, as before.
 */
export const CustomConfigGateModal: React.FC<CustomConfigGateModalProps> = ({
  isOpen,
  onClose,
  onGoExpert,
}) => {
  const { t } = useI18n();

  if (!isOpen) return null;

  // Portal to document.body: glass ancestors (backdrop-filter) would
  // otherwise become the containing block for `fixed` and confine the
  // backdrop to the card instead of the window (see CustomProfileModal).
  return createPortal(
    <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 modal-backdrop-enter">
      <div className="m-auto w-full max-w-md rounded-card border border-line bg-card p-6 shadow-2xl modal-panel-enter">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-amber-500/10 border border-amber-500/25 text-amber-400">
              <Settings2 className="h-5 w-5" />
            </div>
            <h2 className="text-base font-bold text-ink">{t.strategies.customGateTitle}</h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-control p-1.5 text-ink4 hover:bg-hovert hover:text-ink2 cursor-pointer"
            aria-label={t.strategies.customGateCancel}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <p className="mt-4 text-sm leading-relaxed text-ink3">
          {t.strategies.customGateText}
        </p>

        <div className="mt-5 flex justify-end gap-2">
          <button type="button" onClick={onClose} className="t-btn t-btn-ghost">
            {t.strategies.customGateCancel}
          </button>
          <button type="button" onClick={onGoExpert} className="t-btn t-btn-primary">
            {t.strategies.customGateGoExpert}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
};
