import React, { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DiagnosticsCard } from "../DiagnosticsCard";
import { NetworkDoctorModal } from "../NetworkDoctorModal";
import { ClassifierModal, ClassifierTargetRequest } from "../ClassifierModal";
import { ClassifierSession, DiagnosticsReport, ProbeStatus } from "../../types";
import { NavTab } from "../Sidebar";
import { Wrench, HelpCircle, X } from "lucide-react";
import { useI18n } from "../../i18n";
import { isAutoTuneModalOpen, isProbeCycleRunning } from "../../utils/watchdog";

interface DiagnosticsViewProps {
  diagnostics: DiagnosticsReport;
  onRefreshProbes: () => void;
  isRefreshingProbes: boolean;
  onSelectAndStartStrategy: (id: string) => void;
  onNavigateTab: (tab: NavTab) => void;
}

/** Classification targets mirrored from the diagnostics report. Names
 * are proper nouns, shown as-is in every locale (same precedent as the
 * auto-tune results list). */
const CLASSIFIER_TARGETS: { id: string; name: string; key: "youtube" | "discord" | "discord_voice" | "telegram" | "twitter" }[] = [
  { id: "youtube", name: "YouTube", key: "youtube" },
  { id: "discord", name: "Discord", key: "discord" },
  { id: "discord_voice", name: "Discord Voice", key: "discord_voice" },
  { id: "telegram", name: "Telegram", key: "telegram" },
  { id: "twitter", name: "X (Twitter)", key: "twitter" },
];

const statusDot = (status: ProbeStatus) =>
  status === "operational"
    ? "bg-emerald-400"
    : status === "degraded"
      ? "bg-amber-400"
      : status === "blocked"
        ? "bg-rose-400"
        : "bg-slate-500";

export const DiagnosticsView: React.FC<DiagnosticsViewProps> = React.memo(
  ({ diagnostics, onRefreshProbes, isRefreshingProbes, onSelectAndStartStrategy, onNavigateTab }) => {
    const { t } = useI18n();
    const [isDoctorOpen, setIsDoctorOpen] = useState(false);
    const [classifierReq, setClassifierReq] = useState<ClassifierTargetRequest | null>(null);
    const [isClassifierOpen, setIsClassifierOpen] = useState(false);
    const [classifierBusyNote, setClassifierBusyNote] = useState(false);
    const [customDomain, setCustomDomain] = useState("");
    const [restoredNotice, setRestoredNotice] = useState<string | null>(null);

    // Dead-man restore notice (§4.3/§6): shown once, when the backend
    // recovered an interrupted classification at app start. The
    // backend performs that restore ~1.5 s after launch, and this
    // view can mount before it lands — take_classifier_restored then
    // returns null and a single read would lose the notice for good.
    // Poll briefly (bounded: 10 tries, 1 s apart) until the record
    // arrives or the window passes.
    useEffect(() => {
      let cancelled = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      (async () => {
        for (let attempt = 0; attempt < 10 && !cancelled; attempt++) {
          let session: ClassifierSession | null;
          try {
            session = await invoke<ClassifierSession | null>("take_classifier_restored");
          } catch {
            return; /* no notice without a record */
          }
          if (cancelled) return;
          if (session) {
            if (!session.was_engine_running) return;
            if (!session.strategy_id) {
              // Custom-args run (§4.3): there is no strategy id to
              // name — the engine was returned to the saved strategy.
              if (!cancelled) {
                setRestoredNotice(t.classifier.restoredNoticeCustom);
              }
              return;
            }
            let name: string = session.strategy_id;
            try {
              const list = await invoke<{ id: string; name: string }[]>("list_strategies");
              const found = list.find((s) => s.id === session.strategy_id);
              if (found) name = found.name;
            } catch {
              /* the id itself is a fine fallback */
            }
            if (!cancelled) {
              setRestoredNotice(t.classifier.restoredNotice.replace("{strategy}", name));
            }
            return;
          }
          await new Promise<void>((resolve) => {
            timer = setTimeout(resolve, 1000);
          });
        }
      })();
      return () => {
        cancelled = true;
        if (timer) clearTimeout(timer);
      };
    }, [t]);

    const startClassify = useCallback((req: ClassifierTargetRequest) => {
      // §4.2: never start under another probe cycle or the auto-tune
      // modal — say so instead of queueing.
      if (isProbeCycleRunning() || isAutoTuneModalOpen()) {
        setClassifierBusyNote(true);
        return;
      }
      setClassifierBusyNote(false);
      setClassifierReq(req);
      setIsClassifierOpen(true);
    }, []);

    return (
      <div className="space-y-6 pb-6">
        {/* Dead-man restore notice */}
        {restoredNotice && (
          <div className="flex items-start justify-between gap-3 rounded-panel border border-line bg-card p-4 stagger-item" style={{ "--stagger-i": 0 } as React.CSSProperties}>
            <p className="text-[11px] leading-relaxed text-ink2">{restoredNotice}</p>
            <button
              type="button"
              onClick={() => setRestoredNotice(null)}
              className="rounded-control p-1 text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer shrink-0"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* Banner with Doctor Quick Action */}
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 rounded-panel border border-warnline bg-warnwash p-4 stagger-item" style={{ "--stagger-i": 0 } as React.CSSProperties}>
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-amber-500/10 border border-amber-500/25 text-amber-400">
              <Wrench className="h-4 w-4" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-ink flex items-center gap-2">
                <span>{t.doctor.title}</span>
                <span className="rounded-chip bg-amber-500/15 border border-amber-500/25 px-1.5 py-0.5 text-[9px] font-medium text-amber-300">
                  Self-Healing
                </span>
              </h3>
              <p className="text-[11px] text-ink4 mt-0.5">
                {t.doctor.bannerDesc}
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setIsDoctorOpen(true)}
            className="flex items-center justify-center gap-2 rounded-control bg-amber-500 hover:bg-amber-400 text-slate-950 font-semibold px-4 py-2 text-xs transition-all active:scale-[0.98] cursor-pointer shrink-0"
          >
            <Wrench className="h-3.5 w-3.5" />
            <span>{t.doctor.bannerBtn}</span>
          </button>
        </div>

        {/* Block classifier: why a site does not open (provider side) */}
        <div className="rounded-panel border border-line bg-card p-4 stagger-item" style={{ "--stagger-i": 1 } as React.CSSProperties}>
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-panel bg-accent/10 border border-accent/25 text-accent">
              <HelpCircle className="h-4 w-4" />
            </div>
            <div>
              <h3 className="text-xs font-semibold text-ink">{t.classifier.bannerTitle}</h3>
              <p className="text-[11px] text-ink4 mt-0.5">{t.classifier.bannerDesc}</p>
            </div>
          </div>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="text-[11px] text-ink4">{t.classifier.targetLabel}:</span>
            {CLASSIFIER_TARGETS.map((target) => {
              const status = diagnostics[target.key]?.status ?? "checking";
              return (
                <button
                  key={target.id}
                  type="button"
                  onClick={() => startClassify({ target: target.id, targetName: target.name })}
                  className="flex items-center gap-1.5 rounded-control border border-linestrong bg-raise px-3 py-1.5 text-[11px] font-semibold text-ink2 hover:bg-linestrong transition-colors cursor-pointer"
                >
                  <span className={`h-1.5 w-1.5 rounded-full ${statusDot(status)}`} />
                  {target.name}
                </button>
              );
            })}
          </div>

          <form
            className="mt-3 flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              const value = customDomain.trim();
              if (value) startClassify({ target: value, targetName: value });
            }}
          >
            <input
              type="text"
              value={customDomain}
              onChange={(e) => setCustomDomain(e.target.value)}
              placeholder={t.classifier.customPlaceholder}
              className="min-w-0 flex-1 rounded-control border border-linestrong bg-raise px-3 py-1.5 text-[11px] text-ink placeholder:text-ink4 outline-none focus:border-accent"
            />
            <button
              type="submit"
              className="rounded-control bg-accent px-3 py-1.5 text-[11px] font-semibold text-white hover:opacity-90 cursor-pointer shrink-0"
            >
              {t.classifier.customBtn}
            </button>
          </form>

          {classifierBusyNote && (
            <p className="mt-2 text-[11px] text-amber-300">{t.classifier.busyNote}</p>
          )}
        </div>

        {/* Real-Time Probes & Auto-Tune Card */}
        <DiagnosticsCard
          report={diagnostics}
          onRefresh={onRefreshProbes}
          isRefreshing={isRefreshingProbes}
          onSelectAndStartStrategy={onSelectAndStartStrategy}
          onNavigateTab={onNavigateTab}
        />

        {/* Network Doctor Modal */}
        <NetworkDoctorModal
          isOpen={isDoctorOpen}
          onClose={() => setIsDoctorOpen(false)}
          onSuccessRefresh={onRefreshProbes}
        />

        {/* Block Classifier Modal */}
        <ClassifierModal
          open={isClassifierOpen}
          request={classifierReq}
          onClose={() => setIsClassifierOpen(false)}
          onOpenStrategies={() => onNavigateTab("strategies")}
        />
      </div>
    );
  }
);
