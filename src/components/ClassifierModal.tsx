import React, { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { HelpCircle, Loader2, X } from "lucide-react";
import {
  ClassifierStage,
  Diagnosis,
  DiagnosisClass,
  SupervisorStatusResponse,
} from "../types";
import { useI18n } from "../i18n";
import { formatErrorMessage } from "../utils/error";
import {
  isProbeCycleRunning,
  setClassifierModalOpenFlag,
  setProbeCycleRunning,
} from "../utils/watchdog";

/** What to classify: a known target id ("youtube", "discord", ...) or a
 * user-supplied domain; `targetName` is the human name for the texts. */
export interface ClassifierTargetRequest {
  target: string;
  targetName: string;
}

interface ClassifierModalProps {
  open: boolean;
  request: ClassifierTargetRequest | null;
  onClose: () => void;
  /** Navigate to the strategies tab (Sni action). */
  onOpenStrategies: () => void;
  /** Re-run auto-tune (Sni action; only where an auto-tune exists —
   * the post-auto-tune hint passes it, the manual entry does not). */
  onRerunAutoTune?: () => void;
}

type Phase = "loading" | "confirm" | "running" | "cancelling" | "result" | "error";

interface EngineContext {
  running: boolean;
  customArgs: boolean;
  strategyName: string | null;
}

const CLASS_TEXT: Record<DiagnosisClass, { title: "openTitle" | "dnsTitle" | "sniTitle" | "ipTitle" | "serverDownTitle" | "unknownTitle" | "noNetworkTitle"; body: "openBody" | "dnsBody" | "sniBody" | "ipBody" | "serverDownBody" | "unknownBody" | "noNetworkBody" }> = {
  open: { title: "openTitle", body: "openBody" },
  dns: { title: "dnsTitle", body: "dnsBody" },
  sni: { title: "sniTitle", body: "sniBody" },
  ip: { title: "ipTitle", body: "ipBody" },
  server_down: { title: "serverDownTitle", body: "serverDownBody" },
  unknown: { title: "unknownTitle", body: "unknownBody" },
  no_network: { title: "noNetworkTitle", body: "noNetworkBody" },
};

/**
 * Block classifier UI (design §5–§6): confirmation with the engine-pause
 * warning, stage progress, cancellation, and the diagnosis card with
 * per-class actions. The whole cycle is one Rust command; this modal
 * only starts it, listens for stage events, and renders the verdict.
 */
export const ClassifierModal: React.FC<ClassifierModalProps> = ({
  open,
  request,
  onClose,
  onOpenStrategies,
  onRerunAutoTune,
}) => {
  const { t } = useI18n();
  const [phase, setPhase] = useState<Phase>("loading");
  const [engine, setEngine] = useState<EngineContext>({ running: false, customArgs: false, strategyName: null });
  const [stage, setStage] = useState<ClassifierStage>("control");
  const [diagnosis, setDiagnosis] = useState<Diagnosis | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [dnsApplied, setDnsApplied] = useState(false);
  const [dnsBusy, setDnsBusy] = useState(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Publish "classifier modal open" (§4.2), mirroring the auto-tune
  // modal flag: while this modal is up — even on its confirmation
  // screen, before any cycle starts — the watchdog must not begin an
  // escalation cycle that would fight the classification over the
  // engine.
  useEffect(() => {
    setClassifierModalOpenFlag(open);
    return () => setClassifierModalOpenFlag(false);
  }, [open ]);

  // On open: learn the engine state for the confirmation text (§6).
  useEffect(() => {
    if (!open || !request) return;
    setPhase("loading");
    setDiagnosis(null);
    setErrorText(null);
    setDnsApplied(false);
    setStage("control");
    let cancelled = false;
    (async () => {
      try {
        const st = await invoke<SupervisorStatusResponse>("get_supervisor_status");
        let strategyName: string | null = st.supervisor.active_strategy ?? null;
        if (strategyName) {
          try {
            const list = await invoke<{ id: string; name: string }[]>("list_strategies");
            const found = list.find((s) => s.id === strategyName);
            if (found) strategyName = found.name;
          } catch {
            /* the id itself is a fine fallback */
          }
        }
        if (cancelled || !mountedRef.current) return;
        setEngine({
          running: st.supervisor.state === "running",
          customArgs: st.supervisor.active_custom_args === true,
          strategyName,
        });
        setPhase("confirm");
      } catch (err) {
        if (cancelled || !mountedRef.current) return;
        setErrorText(formatErrorMessage(err));
        setPhase("error");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, request]);

  // Stage progress events while a cycle runs.
  useEffect(() => {
    if (phase !== "running" && phase !== "cancelling") return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<ClassifierStage>("classifier-progress", (e) => {
      if (!disposed && mountedRef.current) setStage(e.payload);
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [phase]);

  const runClassification = useCallback(async () => {
    if (!request) return;
    // Re-check the probe lock at start time (§4.2): the watchdog or
    // the auto-tune may have claimed it while this modal sat on its
    // confirmation screen. Starting anyway would interleave engine
    // restarts with the other cycle — show the busy note instead.
    if (isProbeCycleRunning()) {
      setErrorText(t.classifier.busyNote);
      setPhase("error");
      return;
    }
    setPhase("running");
    setStage("control");
    // The shared probe lock (§4.2): the watchdog and auto-tune stay out
    // while the classifier owns the engine.
    setProbeCycleRunning(true);
    try {
      const d = await invoke<Diagnosis>("classify_target", {
        target: request.target,
        targetName: request.targetName,
      });
      if (!mountedRef.current) return;
      setDiagnosis(d);
      setPhase("result");
    } catch (err) {
      const msg = formatErrorMessage(err);
      if (!mountedRef.current) return;
      if (msg.includes("CLASSIFIER_CANCELLED")) {
        // Cancellation is not an error and not a diagnosis (§3.4):
        // the progress just goes away.
        onClose();
        return;
      }
      if (msg.includes("CLASSIFIER_BUSY")) {
        setErrorText(t.classifier.busyNote);
      } else {
        setErrorText(msg);
      }
      setPhase("error");
    } finally {
      setProbeCycleRunning(false);
    }
  }, [request, onClose, t]);

  const handleCancelRun = useCallback(async () => {
    setPhase("cancelling");
    try {
      await invoke("cancel_classify_target");
    } catch {
      /* the cycle resolves with the cancellation sentinel either way */
    }
  }, []);

  const handleApplyDns = useCallback(async () => {
    setDnsBusy(true);
    try {
      await invoke("set_system_dns", { providerId: "cloudflare", scope: "full" });
      if (mountedRef.current) setDnsApplied(true);
    } catch (err) {
      if (mountedRef.current) setErrorText(formatErrorMessage(err));
    } finally {
      if (mountedRef.current) setDnsBusy(false);
    }
  }, []);

  if (!open || !request) return null;

  const targetName = request.targetName;
  const confirmText = !engine.running
    ? t.classifier.confirmStopped.replace("{target}", targetName)
    : engine.customArgs || !engine.strategyName
      ? t.classifier.confirmRunningCustom.replace("{target}", targetName)
      : t.classifier.confirmRunning
          .replace("{target}", targetName)
          .replace("{strategy}", engine.strategyName ?? "");

  const stageText =
    stage === "control"
      ? t.classifier.stageControl
      : stage === "resolve"
        ? t.classifier.stageResolve
        : stage === "sys_path" || stage === "addresses"
          ? t.classifier.stageThrough
          : stage === "tls"
            ? t.classifier.stageTls
            : t.classifier.stageVerdict;

  const busy = phase === "running" || phase === "cancelling";

  return createPortal(
    <div className="fixed inset-0 z-50 flex bg-overlay backdrop-blur-md p-4 overflow-y-auto no-scrollbar modal-backdrop-enter">
      <div className="m-auto w-full max-w-lg rounded-card border border-line bg-card p-5 shadow-2xl modal-panel-enter">
        <div className="flex items-center justify-between pb-3 border-b border-raise">
          <div className="flex items-center gap-2">
            <HelpCircle className="h-5 w-5 text-accent" />
            <h3 className="text-sm font-semibold text-ink">{t.classifier.modalTitle}</h3>
          </div>
          <button
            type="button"
            onClick={busy ? handleCancelRun : onClose}
            className="rounded-control p-1 text-ink4 hover:text-ink hover:bg-raise transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {phase === "loading" && (
          <div className="mt-4 flex items-center gap-2 text-xs text-ink2">
            <Loader2 className="h-4 w-4 animate-spin text-accent" />
            <span>{t.classifier.stageControl}</span>
          </div>
        )}

        {phase === "confirm" && (
          <>
            <p className="mt-3 text-xs leading-relaxed text-ink2">{confirmText}</p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
              >
                {t.classifier.cancelBtn}
              </button>
              <button
                type="button"
                onClick={runClassification}
                className="rounded-control bg-accent px-4 py-1.5 text-xs font-semibold text-white hover:opacity-90 cursor-pointer"
              >
                {t.classifier.confirmBtn}
              </button>
            </div>
          </>
        )}

        {busy && (
          <>
            <div className="mt-4 flex items-center gap-2 text-xs text-ink2">
              <Loader2 className="h-4 w-4 animate-spin text-accent" />
              <span>{stageText}</span>
            </div>
            <div className="mt-4 flex justify-end">
              <button
                type="button"
                onClick={handleCancelRun}
                disabled={phase === "cancelling"}
                className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer disabled:opacity-50"
              >
                {t.classifier.cancelBtn}
              </button>
            </div>
          </>
        )}

        {phase === "error" && (
          <>
            <p className="mt-3 text-xs leading-relaxed text-rose-400">{errorText}</p>
            <div className="mt-4 flex justify-end">
              <button
                type="button"
                onClick={onClose}
                className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
              >
                {t.classifier.okBtn}
              </button>
            </div>
          </>
        )}

        {phase === "result" && diagnosis && (
          <>
            <p className="mt-3 text-[11px] leading-relaxed text-ink4">{t.classifier.resultHeader}</p>
            <h4 className="mt-2 text-sm font-semibold text-ink">
              {t.classifier[CLASS_TEXT[diagnosis.class].title].replace("{target}", diagnosis.target_name)}
            </h4>
            <p className="mt-1.5 text-xs leading-relaxed text-ink2">
              {t.classifier[CLASS_TEXT[diagnosis.class].body].replace("{target}", diagnosis.target_name)}
            </p>

            {diagnosis.partial_mode && diagnosis.class !== "no_network" && (
              <p className="mt-2 text-[11px] leading-relaxed text-amber-300/90">{t.classifier.partialNote}</p>
            )}
            {dnsApplied && (
              <p className="mt-2 text-[11px] leading-relaxed text-emerald-300">{t.classifier.dnsApplied}</p>
            )}
            {errorText && <p className="mt-2 text-[11px] text-rose-400">{errorText}</p>}

            <details className="mt-3 rounded-panel border border-raise bg-cardin/40 p-3">
              <summary className="cursor-pointer text-[11px] font-semibold text-ink3">{t.classifier.detailsTitle}</summary>
              <dl className="mt-2 space-y-1 font-mono text-[11px] text-ink4 break-all">
                <div>class: {diagnosis.class}</div>
                <div>context: {diagnosis.context}</div>
                <div>ctrl_alive: {String(diagnosis.details.ctrl_alive)}</div>
                <div>sys_addrs: {diagnosis.details.sys_addrs.join(", ") || "—"}</div>
                <div>ref_addrs: {diagnosis.details.ref_addrs.join(", ") || "—"}</div>
                <div>
                  ref_evidence_grade: {String(diagnosis.details.ref_evidence_grade)}; corroborated:{" "}
                  {String(diagnosis.details.ref_corroborated)}
                </div>
                <div>ctrl_neutral_alive: {String(diagnosis.details.ctrl_neutral_alive)}</div>
                <div>deadline_hit: {String(diagnosis.details.deadline_hit)}</div>
                <div>
                  cert_valid: {diagnosis.details.cert_valid === null ? "—" : String(diagnosis.details.cert_valid)}
                  {diagnosis.details.cert_subject ? ` (${diagnosis.details.cert_subject})` : ""}
                </div>
                <div>sys_https: {diagnosis.details.sys_https.join(" / ") || "—"}</div>
                {diagnosis.details.addr_outcomes.map((line) => (
                  <div key={line}>{line}</div>
                ))}
              </dl>
            </details>

            <div className="mt-4 flex flex-wrap justify-end gap-2">
              {diagnosis.class === "dns" && !dnsApplied && (
                <button
                  type="button"
                  onClick={handleApplyDns}
                  disabled={dnsBusy}
                  className="rounded-control bg-accent px-4 py-1.5 text-xs font-semibold text-white hover:opacity-90 cursor-pointer disabled:opacity-50"
                >
                  {t.classifier.dnsActionBtn}
                </button>
              )}
              {diagnosis.class === "sni" && (
                <>
                  <button
                    type="button"
                    onClick={() => {
                      onClose();
                      onOpenStrategies();
                    }}
                    className="rounded-control bg-accent px-4 py-1.5 text-xs font-semibold text-white hover:opacity-90 cursor-pointer"
                  >
                    {t.classifier.sniStrategiesBtn}
                  </button>
                  {onRerunAutoTune && (
                    <button
                      type="button"
                      onClick={() => {
                        onClose();
                        onRerunAutoTune();
                      }}
                      className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
                    >
                      {t.classifier.sniRerunBtn}
                    </button>
                  )}
                </>
              )}
              {diagnosis.class === "unknown" && (
                <button
                  type="button"
                  onClick={() => setPhase("confirm")}
                  className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
                >
                  {t.classifier.retryBtn}
                </button>
              )}
              <button
                type="button"
                onClick={onClose}
                className="rounded-control border border-linestrong bg-raise px-4 py-1.5 text-xs font-semibold text-ink2 hover:bg-linestrong cursor-pointer"
              >
                {t.classifier.okBtn}
              </button>
            </div>
          </>
        )}
      </div>
    </div>,
    document.body,
  );
};
