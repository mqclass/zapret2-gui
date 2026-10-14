import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LuaStrategy } from "../types";
import { useI18n } from "../i18n";
import { X, Sparkles, Terminal, Check, AlertCircle, Tv, Mic, Globe, Zap, Lightbulb } from "lucide-react";

interface CustomProfileModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (strategy: LuaStrategy) => Promise<void>;
  editingStrategy: LuaStrategy | null;
}

export const CustomProfileModal: React.FC<CustomProfileModalProps> = ({
  isOpen,
  onClose,
  onSave,
  editingStrategy,
}) => {
  const { t } = useI18n();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [engine, setEngine] = useState<"winws" | "winws2">("winws");
  const [argsInput, setArgsInput] = useState("");
  const [parsedTokens, setParsedTokens] = useState<string[]>([]);
  const [isParsing, setIsParsing] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showCheatsheet, setShowCheatsheet] = useState<boolean>(false);

  const applyQuickTemplate = (type: "youtube" | "discord" | "general" | "combo") => {
    setEngine("winws");
    let templateArgs = "";
    if (type === "youtube") {
      setName((prev) => (prev.trim() ? prev : "YouTube 4K 60FPS"));
      setDescription("Оптимизировано для обхода замедлений YouTube (QUIC + TCP fake/multisplit seqovl=681)");
      templateArgs = `--wf-tcp=80,443 --wf-udp=443 --filter-udp=443 --hostlist="lists/list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin" --new --filter-tcp=443 --hostlist="lists/list-google.txt" --ip-id=zero --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin"`;
    } else if (type === "discord") {
      setName((prev) => (prev.trim() ? prev : "Discord Голос & Стримы"));
      setDescription("Обход голосовых шлюзов Discord (UDP 50000-50100) и медиа-серверов");
      templateArgs = `--wf-tcp=2053,2083,2087,2096,8443 --wf-udp=19294-19344,50000-50100 --filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-fake-stun="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-repeats=6 --new --filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin"`;
    } else if (type === "general") {
      setName((prev) => (prev.trim() ? prev : "Обход сайтов РКН"));
      setDescription("Универсальная маскировка блокировок сайтов (tls_max_ru, seqovl=664)");
      templateArgs = `--wf-tcp=80,443 --filter-tcp=80,443 --hostlist="lists/list-general.txt" --hostlist-exclude="lists/list-exclude.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=664 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-tls="bin/stun2.bin" --dpi-desync-fake-tls="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-http="bin/tls_clienthello_max_ru.bin"`;
    } else if (type === "combo") {
      setName((prev) => (prev.trim() ? prev : "Комбо (YouTube + Discord + Сайты)"));
      setDescription("Полный комплексный обход всех видов блокировок ТСПУ");
      templateArgs = `--wf-tcp=80,443,2053,2083,2087,2096,8443 --wf-udp=443,19294-19344,50000-50100 --filter-udp=443 --hostlist="lists/list-general.txt" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic="bin/quic_initial_www_google_com.bin" --new --filter-udp=19294-19344,50000-50100 --filter-l7=discord,stun --dpi-desync=fake --dpi-desync-fake-discord="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-fake-stun="bin/ACTIVE_DISCORD_UDP.bin" --dpi-desync-repeats=6 --new --filter-tcp=2053,2083,2087,2096,8443 --hostlist-domains=discord.media --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin" --new --filter-tcp=443 --hostlist="lists/list-google.txt" --ip-id=zero --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_www_google_com.bin" --dpi-desync-fake-tls="bin/tls_clienthello_www_google_com.bin" --new --filter-tcp=80,443 --hostlist="lists/list-general.txt" --hostlist-exclude="lists/list-exclude.txt" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=664 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-split-seqovl-pattern="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-tls="bin/stun2.bin" --dpi-desync-fake-tls="bin/tls_clienthello_max_ru.bin" --dpi-desync-fake-http="bin/tls_clienthello_max_ru.bin"`;
    }
    setArgsInput(templateArgs);
    setParsedTokens([]);
  };

  useEffect(() => {
    if (editingStrategy) {
      setName(editingStrategy.name);
      setDescription(editingStrategy.description || "");
      setEngine(editingStrategy.engine);
      if (editingStrategy.custom_args && editingStrategy.custom_args.length > 0) {
        setArgsInput(editingStrategy.custom_args.join(" "));
        setParsedTokens(editingStrategy.custom_args);
      } else {
        setArgsInput("");
        setParsedTokens([]);
      }
    } else {
      setName("");
      setDescription("");
      setEngine("winws");
      setArgsInput("");
      setParsedTokens([]);
    }
    setError(null);
  }, [editingStrategy, isOpen]);

  if (!isOpen) return null;

  const handleParse = async () => {
    if (!argsInput.trim()) {
      setParsedTokens([]);
      return;
    }
    setIsParsing(true);
    setError(null);
    try {
      const tokens = await invoke<string[]>("parse_args_string", {
        raw: argsInput.trim(),
      });
      setParsedTokens(tokens);
    } catch (err: any) {
      console.error("Failed to parse args:", err);
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsParsing(false);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError(`${t.modal.nameLabel} ${t.modal.fieldRequired}`);
      return;
    }

    setIsSaving(true);
    setError(null);

    try {
      let tokens = parsedTokens;
      if (tokens.length === 0 && argsInput.trim()) {
        tokens = await invoke<string[]>("parse_args_string", {
          raw: argsInput.trim(),
        });
      }

      const id = editingStrategy ? editingStrategy.id : `custom-${Date.now()}`;
      const safeFilename = `${name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, "_")}.conf`;

      const strategy: LuaStrategy = {
        id,
        name: name.trim(),
        filename: safeFilename,
        path: editingStrategy ? editingStrategy.path : `configs/${id}.json`,
        size_bytes: 0,
        description: description.trim(),
        engine,
        category: "custom",
        custom_args: tokens.length > 0 ? tokens : null,
      };

      await onSave(strategy);
      onClose();
    } catch (err: any) {
      console.error("Failed to save custom profile:", err);
      setError(typeof err === "string" ? err : JSON.stringify(err));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4 overflow-y-auto">
      <div className="relative w-full max-w-2xl rounded-2xl border border-slate-800 bg-slate-900 p-6 shadow-2xl my-8">
        <div className="flex items-center justify-between pb-4 border-b border-slate-800">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-blue-500/10 text-blue-400 border border-blue-500/20">
              <Sparkles className="h-4 w-4" />
            </div>
            <div>
              <h3 className="text-base font-semibold text-white">
                {editingStrategy ? t.modal.editTitle : t.modal.createTitle}
              </h3>
              <p className="text-xs text-slate-400">
                {editingStrategy ? editingStrategy.id : "configs/custom_profiles.json"}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-slate-400 hover:bg-slate-800 hover:text-white transition-colors cursor-pointer"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {error && (
          <div className="mt-4 flex items-center gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-300">
            <AlertCircle className="h-4 w-4 shrink-0 text-rose-400" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSave} className="mt-4 space-y-4">
          <div>
            <label className="block text-xs font-semibold text-slate-300 mb-1">
              {t.modal.nameLabel} <span className="text-rose-400">*</span>
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t.modal.namePlaceholder}
              required
              className="w-full rounded-xl border border-slate-700 bg-slate-950 px-3.5 py-2 text-xs text-white placeholder-slate-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>

          <div>
            <label className="block text-xs font-semibold text-slate-300 mb-1">
              {t.modal.descLabel}
            </label>
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t.modal.descPlaceholder}
              className="w-full rounded-xl border border-slate-700 bg-slate-950 px-3.5 py-2 text-xs text-white placeholder-slate-500 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>

          <div>
            <label className="block text-xs font-semibold text-slate-300 mb-1.5">
              {t.modal.engineLabel}
            </label>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              <label
                className={`flex items-start gap-3 rounded-xl border p-3 cursor-pointer transition-all ${
                  engine === "winws"
                    ? "border-indigo-500/70 bg-indigo-500/10 text-white"
                    : "border-slate-800 bg-slate-950/60 text-slate-400 hover:border-slate-700"
                }`}
              >
                <input
                  type="radio"
                  name="engine"
                  value="winws"
                  checked={engine === "winws"}
                  onChange={() => setEngine("winws")}
                  className="mt-0.5 text-indigo-500"
                />
                <div>
                  <div className="text-xs font-semibold text-indigo-300">
                    winws.exe (Flowseal / v1)
                  </div>
                  <div className="text-[11px] text-slate-400 mt-0.5 leading-snug">
                    {t.modal.engineFlowsealDesc}
                  </div>
                </div>
              </label>

              <label
                className={`flex items-start gap-3 rounded-xl border p-3 cursor-pointer transition-all ${
                  engine === "winws2"
                    ? "border-sky-500/70 bg-sky-500/10 text-white"
                    : "border-slate-800 bg-slate-950/60 text-slate-400 hover:border-slate-700"
                }`}
              >
                <input
                  type="radio"
                  name="engine"
                  value="winws2"
                  checked={engine === "winws2"}
                  onChange={() => setEngine("winws2")}
                  className="mt-0.5 text-sky-500"
                />
                <div>
                  <div className="text-xs font-semibold text-sky-300">
                    winws2.exe (zapret2 / v2)
                  </div>
                  <div className="text-[11px] text-slate-400 mt-0.5 leading-snug">
                    {t.modal.engineZapret2Desc}
                  </div>
                </div>
              </label>
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-semibold text-slate-300">
                {t.modal.argsLabel}
              </label>
              <button
                type="button"
                onClick={handleParse}
                disabled={isParsing || !argsInput.trim()}
                className="text-[11px] font-medium text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-1 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
              >
                <Terminal className="h-3 w-3" />
                <span>{isParsing ? "..." : t.modal.parseBtn}</span>
              </button>
            </div>

            {/* Quick Templates Bar */}
            <div className="mb-2">
              <div className="text-[11px] font-medium text-slate-400 mb-1 flex items-center gap-1.5">
                <Sparkles className="h-3 w-3 text-amber-400" />
                <span>{t.modal.quickTemplates}:</span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                <button
                  type="button"
                  onClick={() => applyQuickTemplate("youtube")}
                  className="flex items-center gap-1.5 rounded-lg border border-red-500/30 bg-red-950/30 px-2.5 py-1 text-[11px] font-medium text-red-300 hover:bg-red-900/40 hover:border-red-500/50 transition-all cursor-pointer"
                >
                  <Tv className="h-3 w-3 text-red-400" />
                  <span>{t.modal.templateYoutube}</span>
                </button>
                <button
                  type="button"
                  onClick={() => applyQuickTemplate("discord")}
                  className="flex items-center gap-1.5 rounded-lg border border-indigo-500/30 bg-indigo-950/30 px-2.5 py-1 text-[11px] font-medium text-indigo-300 hover:bg-indigo-900/40 hover:border-indigo-500/50 transition-all cursor-pointer"
                >
                  <Mic className="h-3 w-3 text-indigo-400" />
                  <span>{t.modal.templateDiscord}</span>
                </button>
                <button
                  type="button"
                  onClick={() => applyQuickTemplate("general")}
                  className="flex items-center gap-1.5 rounded-lg border border-emerald-500/30 bg-emerald-950/30 px-2.5 py-1 text-[11px] font-medium text-emerald-300 hover:bg-emerald-900/40 hover:border-emerald-500/50 transition-all cursor-pointer"
                >
                  <Globe className="h-3 w-3 text-emerald-400" />
                  <span>{t.modal.templateGeneral}</span>
                </button>
                <button
                  type="button"
                  onClick={() => applyQuickTemplate("combo")}
                  className="flex items-center gap-1.5 rounded-lg border border-purple-500/30 bg-purple-950/30 px-2.5 py-1 text-[11px] font-medium text-purple-300 hover:bg-purple-900/40 hover:border-purple-500/50 transition-all cursor-pointer"
                >
                  <Zap className="h-3 w-3 text-purple-400" />
                  <span>{t.modal.templateCombo}</span>
                </button>
              </div>
            </div>

            <textarea
              rows={4}
              value={argsInput}
              onChange={(e) => {
                setArgsInput(e.target.value);
                setParsedTokens([]);
              }}
              placeholder={t.modal.argsPlaceholder}
              className="w-full rounded-xl border border-slate-700 bg-slate-950 px-3.5 py-2.5 font-mono text-xs text-white placeholder-slate-600 focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 leading-relaxed"
            />
            <p className="mt-1 text-[11px] text-slate-400 leading-snug">
              {t.modal.argsHint}
            </p>

            {/* Cheatsheet Expander */}
            <div className="mt-2.5 rounded-xl border border-slate-800 bg-slate-950/80 p-3">
              <button
                type="button"
                onClick={() => setShowCheatsheet(!showCheatsheet)}
                className="flex w-full items-center justify-between text-left text-xs font-semibold text-slate-300 hover:text-white transition-colors cursor-pointer"
              >
                <div className="flex items-center gap-1.5">
                  <Lightbulb className="h-3.5 w-3.5 text-amber-400" />
                  <span>{t.modal.cheatsheetTitle}</span>
                </div>
                <span className="text-[11px] text-slate-500">
                  {showCheatsheet ? "▲ Скрыть" : "▼ Раскрыть"}
                </span>
              </button>

              {showCheatsheet && (
                <div className="mt-2.5 pt-2.5 border-t border-slate-800/80 space-y-2 text-[11px] text-slate-300 leading-relaxed">
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--filter-tcp=80,443 / --filter-udp=443,50000-50100</span>
                    <p className="text-slate-400">Фильтрует сетевые порты. Для YouTube нужен TCP 443 + UDP 443 (QUIC), для Discord Voice — UDP 50000-50100.</p>
                  </div>
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--hostlist="lists/list-general.txt"</span>
                    <p className="text-slate-400">Применяет обход только к доменам из списка. Можно указать несколько раз для разных списков.</p>
                  </div>
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--dpi-desync=fake,multisplit</span>
                    <p className="text-slate-400"><span className="text-white font-medium">fake</span> отправляет поддельный ClientHello/QUIC, <span className="text-white font-medium">multisplit</span> делит реальный TCP пакет на части, сбивая сигнатурный анализ ТСПУ.</p>
                  </div>
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--dpi-desync-split-seqovl=681</span>
                    <p className="text-slate-400">Смещение перекрытия TCP-сегментов. Значение 681 идеально работает под YouTube/Google, а 664 — для остальных сайтов в РФ.</p>
                  </div>
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--dpi-desync-fooling=ts</span>
                    <p className="text-slate-400">Маскировка фейкового пакета таймштампами TCP Timestamp. ТСПУ пропускает фейк, а сервер отбрасывает его и принимает ваши настоящие данные.</p>
                  </div>
                  <div>
                    <span className="font-mono text-cyan-300 font-semibold">--new</span>
                    <p className="text-slate-400">Разделитель блоков правил. Позволяет задать разные методы обхода для YouTube, Discord и заблокированных сайтов в одной строке.</p>
                  </div>
                </div>
              )}
            </div>
          </div>

          {parsedTokens.length > 0 && (
            <div className="rounded-xl border border-slate-800 bg-slate-950 p-3 max-h-36 overflow-y-auto">
              <div className="text-[11px] font-semibold text-slate-300 mb-1.5 flex items-center gap-1.5">
                <Check className="h-3.5 w-3.5 text-emerald-400" />
                <span>{t.modal.parsedTokensTitle} ({parsedTokens.length})</span>
              </div>
              <div className="flex flex-wrap gap-1">
                {parsedTokens.map((tok, idx) => (
                  <span
                    key={idx}
                    className="rounded bg-slate-800/80 border border-slate-700 px-1.5 py-0.5 font-mono text-[10px] text-slate-300 max-w-full break-all"
                  >
                    {tok}
                  </span>
                ))}
              </div>
            </div>
          )}

          <div className="flex items-center justify-end gap-3 pt-4 border-t border-slate-800">
            <button
              type="button"
              onClick={onClose}
              disabled={isSaving}
              className="rounded-xl border border-slate-700 bg-slate-800/60 px-4 py-2 text-xs font-medium text-slate-300 hover:bg-slate-700 hover:text-white transition-colors cursor-pointer"
            >
              {t.modal.cancelBtn}
            </button>
            <button
              type="submit"
              disabled={isSaving}
              className="flex items-center gap-2 rounded-xl bg-blue-600 px-5 py-2 text-xs font-semibold text-white shadow-lg shadow-blue-600/30 hover:bg-blue-500 transition-all cursor-pointer disabled:opacity-50"
            >
              <Check className="h-4 w-4" />
              <span>{isSaving ? "..." : t.modal.saveBtn}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
