import React from "react";
import { createPortal } from "react-dom";
import { useI18n } from "../i18n";
import { X, HelpCircle, CheckCircle2, Zap, Settings, ShieldAlert, Sparkles } from "lucide-react";

interface ConfigGuideModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export const ConfigGuideModal: React.FC<ConfigGuideModalProps> = ({
  isOpen,
  onClose,
}) => {
  const { t } = useI18n();

  if (!isOpen) return null;

  // Portal to document.body: glass ancestors (backdrop-filter) would
  // otherwise become the containing block for `fixed` and confine the
  // backdrop to the card instead of the window (see CustomProfileModal).
  return createPortal(
    <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 overflow-y-auto no-scrollbar modal-backdrop-enter">
      <div className="relative m-auto w-full max-w-2xl rounded-card border border-raise bg-card p-6 shadow-2xl max-h-[90vh] overflow-y-auto modal-panel-enter">
        <div className="flex items-center justify-between pb-4 border-b border-raise">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-control bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
              <HelpCircle className="h-4 w-4" />
            </div>
            <div>
              <h3 className="text-base font-semibold text-ink">
                {t.modal.guideTitle}
              </h3>
              <p className="text-xs text-ink4">
                {t.modal.guideSubtitle}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-control p-1.5 text-ink4 hover:bg-raise hover:text-ink transition-colors cursor-pointer"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="mt-5 space-y-4 text-xs text-ink3 leading-relaxed">
          {/* Card 1: Flowseal General */}
          <div className="rounded-panel border border-emerald-500/30 bg-emerald-950/20 p-4">
            <div className="flex items-center gap-2 mb-2">
              <CheckCircle2 className="h-4 w-4 text-emerald-400" />
              <h4 className="font-semibold text-emerald-300 text-sm">
                {t.modal.guideFlowsealTitle}
              </h4>
            </div>
            <p className="text-ink3">
              {t.modal.guideFlowsealText}
            </p>
            <div className="mt-2.5 flex flex-wrap gap-1.5 font-mono text-[10px]">
              <span className="rounded-chip bg-emerald-500/20 px-2 py-0.5 text-emerald-300 border border-emerald-500/30">
                winws.exe v1
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                YouTube 4K
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                Discord Voice (UDP)
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                seqovl=681
              </span>
            </div>
          </div>

          {/* Card 2: Flowseal ALT11 */}
          <div className="rounded-panel border border-purple-500/30 bg-purple-950/20 p-4">
            <div className="flex items-center gap-2 mb-2">
              <Zap className="h-4 w-4 text-purple-400" />
              <h4 className="font-semibold text-purple-300 text-sm">
                {t.modal.guideFlowsealAltTitle}
              </h4>
            </div>
            <p className="text-ink3">
              {t.modal.guideFlowsealAltText}
            </p>
            <div className="mt-2.5 flex flex-wrap gap-1.5 font-mono text-[10px]">
              <span className="rounded-chip bg-purple-500/20 px-2 py-0.5 text-purple-300 border border-purple-500/30">
                11x Repeats
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                YouTube 4K 60FPS
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                {t.modal.guideTagStrict}
              </span>
            </div>
          </div>

          {/* Card: zapret2 РФ Ultimate */}
          <div className="rounded-panel border border-violet-500/40 bg-violet-950/20 p-4">
            <div className="flex items-center gap-2 mb-2">
              <Sparkles className="h-4 w-4 text-violet-400" />
              <h4 className="font-semibold text-violet-300 text-sm">
                {t.modal.guideUltimateTitle}
              </h4>
            </div>
            <p className="text-ink3">
              {t.modal.guideUltimateText}
            </p>
            <div className="mt-2.5 flex flex-wrap gap-1.5 font-mono text-[10px]">
              <span className="rounded-chip bg-violet-500/20 px-2 py-0.5 text-violet-300 border border-violet-500/30">
                winws2.exe v2
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                YouTube 4K 60FPS
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                Discord Voice (UDP)
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                out-range=n1-4 (Zero CPU)
              </span>
            </div>
          </div>

          {/* Card 3: zapret2 Lua */}
          <div className="rounded-panel border border-fuchsia-500/30 bg-fuchsia-950/20 p-4">
            <div className="flex items-center gap-2 mb-2">
              <Settings className="h-4 w-4 text-fuchsia-400" />
              <h4 className="font-semibold text-fuchsia-300 text-sm">
                {t.modal.guideZapret2Title}
              </h4>
            </div>
            <p className="text-ink3">
              {t.modal.guideZapret2Text}
            </p>
            <div className="mt-2.5 flex flex-wrap gap-1.5 font-mono text-[10px]">
              <span className="rounded-chip bg-fuchsia-500/20 px-2 py-0.5 text-fuchsia-300 border border-fuchsia-500/30">
                winws2.exe v2
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                {t.modal.guideTagLua}
              </span>
              <span className="rounded-chip bg-raise px-2 py-0.5 text-ink3 border border-linestrong">
                {t.modal.guideTagModular}
              </span>
            </div>
          </div>

          {/* Card 4: Custom Configs */}
          <div className="rounded-panel border border-amber-500/30 bg-amber-950/20 p-4">
            <div className="flex items-center gap-2 mb-2">
              <ShieldAlert className="h-4 w-4 text-amber-400" />
              <h4 className="font-semibold text-amber-300 text-sm">
                {t.modal.guideCustomTitle}
              </h4>
            </div>
            <p className="text-ink3">
              {t.modal.guideCustomText}
            </p>
          </div>
        </div>

        <div className="mt-6 flex justify-end pt-4 border-t border-raise">
          <button
            onClick={onClose}
            className="rounded-panel bg-accent-fill px-6 py-2 text-xs font-semibold text-ink shadow-lg shadow-accent-fill/30 hover:bg-accent transition-all cursor-pointer"
          >
            {t.modal.closeBtn}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
};
