import { Language, Translations } from "../i18n/types";
import { LuaStrategy } from "../types";

/**
 * Humanizes frequent engine/supervisor log lines for the simple-view
 * digest ("Последние события", redesign stage 5). The expert LogViewer
 * keeps the raw text — rawness is appropriate there.
 *
 * Every pattern below is grounded in a real string:
 * - supervisor's own log lines: src-tauri/src/engine/supervisor.rs
 *   (`.log(...)` calls, e.g. "Bypass service active (PID: …) …")
 * - engine stdout/stderr forwarded verbatim by the supervisor's log
 *   readers (supervisor.rs, "Spawn asynchronous log readers"), as
 *   observed in the UI stand mocks (~/workspace/stands/zapret-ui/stub.js)
 *   — e.g. "winws2 started with strategy …", "LUA: desync profile
 *   applied for … hostlist entries", "probe: discord gateway latency
 *   spike … ms".
 *
 * Matching is by prefix/pattern, never by exact full-string equality:
 * PIDs, strategy ids, entry counts and latencies vary per run. Unknown
 * lines fall back to the raw text unchanged.
 *
 * Already-human (Russian) supervisor lines — tg-bridge messages, the
 * saved-strategy fallback, admin-rights and Linux nftables notes — need
 * no mapping and fall through untouched.
 */

interface HumanizeContext {
  t: Translations;
  language: Language;
  /** Human strategy name by id; unknown ids pass through as-is. */
  strategyName: (id: string) => string;
}

type Rule = {
  match: RegExp;
  render: (m: RegExpExecArray, ctx: HumanizeContext) => string;
};

/** Service token from a probe line -> display name. */
function probeServiceName(token: string): string {
  const known: Record<string, string> = {
    discord: "Discord",
    youtube: "YouTube",
    telegram: "Telegram",
    twitter: "X",
  };
  const lower = token.toLowerCase();
  if (known[lower]) return known[lower];
  return token.charAt(0).toUpperCase() + token.slice(1);
}

const RULES: Rule[] = [
  // --- Engine stdout/stderr (forwarded verbatim by the supervisor) ---
  {
    // stub.js mock; winws2 announces the strategy it started with.
    match: /^winws2 started with strategy (\S+)$/,
    render: (m, { t, strategyName }) =>
      t.diagTab.logHuman.engineStarted.replace("{strategy}", strategyName(m[1])),
  },
  {
    // stub.js mock; winws2 reports the WinDivert driver is up.
    match: /^WinDivert driver loaded, filter active$/,
    render: (_m, { t }) => t.diagTab.logHuman.driverActive,
  },
  {
    // stub.js mock; the Lua desync profile reports its hostlist size.
    match: /^LUA: desync profile applied for (\d+) hostlist entries$/,
    render: (m, { t, language }) =>
      t.diagTab.logHuman.desyncApplied.replace(
        "{n}",
        Number(m[1]).toLocaleString(language),
      ),
  },
  {
    // stub.js mock; probe watchdog notes a latency spike.
    match: /^probe: (\S+) gateway latency spike (\d+) ms$/,
    render: (m, { t }) =>
      t.diagTab.logHuman.probeLatencySpike
        .replace("{service}", probeServiceName(m[1]))
        .replace("{ms}", m[2]),
  },
  // --- Supervisor lifecycle (supervisor.rs) ---
  {
    // supervisor.rs:406 — version varies, so match the shape.
    match: /^zapret2 v\d+(\.\d+)* initialized\. WinDivert and WFP subsystem ready\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.engineReady,
  },
  {
    // supervisor.rs:407
    match: /^Ready to start DPI bypass\. Select a strategy to begin\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.pickStrategy,
  },
  {
    // supervisor.rs:798
    match: /^Switching config: stopping previous engine \(PID \d+\)$/,
    render: (_m, { t }) => t.diagTab.logHuman.switchingStrategy,
  },
  {
    // supervisor.rs:804
    match: /^Initiating zapret background supervisor\.\.\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.startingBypass,
  },
  {
    // supervisor.rs:1470
    match: /^Initiating shutdown of bypass engine\.\.\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.stoppingBypass,
  },
  {
    // supervisor.rs:1475
    match: /^Executing shutdown protocol on PID \d+$/,
    render: (_m, { t }) => t.diagTab.logHuman.stoppingEngine,
  },
  {
    // supervisor.rs:1508
    match: /^zapret supervisor stopped successfully$/,
    render: (_m, { t }) => t.diagTab.logHuman.bypassOff,
  },
  {
    // supervisor.rs:1241
    match: /^Bypass service active \(PID: \d+\) assigned to Windows Job Object$/,
    render: (_m, { t }) => t.diagTab.logHuman.bypassOn,
  },
  {
    // supervisor.rs:1309 — exit status varies ("exit code: 0", signals…).
    match: /^Bypass process \(PID \d+\) terminated with status: .*$/,
    render: (_m, { t }) => t.diagTab.logHuman.engineExited,
  },
  {
    // supervisor.rs:1016 — exe name is jargon; the strategy is the point.
    match: /^Using bypass engine: \S+ \[(\S+)\]$/,
    render: (m, { t, strategyName }) =>
      t.diagTab.logHuman.startingStrategy.replace("{strategy}", strategyName(m[1])),
  },
  {
    // supervisor.rs:1168 — long raw argument dump; the digest only needs
    // to know the launch settings were applied.
    match: /^Bypass parameters: /,
    render: (_m, { t }) => t.diagTab.logHuman.paramsApplied,
  },
  // --- App split (supervisor.rs:928-979) ---
  {
    match: /^App split disabled: running the standard engine binary$/,
    render: (_m, { t }) => t.diagTab.logHuman.splitDisabled,
  },
  {
    match: /^App split '[^']+' configured but strategy '([^']+)' is Flowseal \(winws\):/,
    render: (m, { t, strategyName }) =>
      t.diagTab.logHuman.splitFlowseal.replace("{strategy}", strategyName(m[1])),
  },
  {
    match: /^App split '[^']+' enforced: using winws2-split\.exe for strategy '[^']+' \(\d+ app paths\)$/,
    render: (_m, { t }) => t.diagTab.logHuman.splitEnforced,
  },
  {
    match: /^App split '[^']+' configured but winws2-split\.exe is not installed yet:/,
    render: (_m, { t }) => t.diagTab.logHuman.splitUnavailable,
  },
  {
    // supervisor.rs:979 (Linux) — same outcome as the Windows
    // "not installed" case: split unavailable, engine runs without it.
    match: /^App split '[^']+' configured but per-app split is not available on Linux/,
    render: (_m, { t }) => t.diagTab.logHuman.splitUnavailable,
  },
  // --- Camouflage transport (supervisor.rs:1073) ---
  {
    match: /^Camouflage transport '([^']+)' applied to launch arguments$/,
    render: (m, { t }) => t.diagTab.logHuman.transportApplied.replace("{transport}", m[1]),
  },
  // --- WinDivert contention / recovery (supervisor.rs:634, 1198, 1384, 1400) ---
  {
    match: /^Direct launch encountered contention \(.*\), executing driver cleanup retry\.\.\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.launchRetry,
  },
  {
    match: /^WinDivert conflict detected: (cleaning up stray engines and retrying once|scheduling one recovery attempt)\.\.\.$/,
    render: (_m, { t }) => t.diagTab.logHuman.windivertConflictRetry,
  },
  {
    match: /^WinDivert filter open failed \(exit code 10\):/,
    render: (_m, { t }) => t.diagTab.logHuman.windivertFilterFailed,
  },
  {
    match: /^WinDivert recovery restart failed: /,
    render: (_m, { t }) => t.diagTab.logHuman.recoveryFailed,
  },
  // --- Auto-hostlist (supervisor.rs:2383) ---
  {
    match: /^Auto-hostlist cleanup failed, continuing start: /,
    render: (_m, { t }) => t.diagTab.logHuman.autoHostlistCleanupFailed,
  },
  // --- Start failures (supervisor.rs:810, 1042, 1060, 1211, 1318) ---
  {
    match: /^Binary discovery failure: /,
    render: (_m, { t }) => t.diagTab.logHuman.binariesNotFound,
  },
  {
    match: /^Failed to build engine arguments: /,
    render: (_m, { t }) => t.diagTab.logHuman.argsBuildFailed,
  },
  {
    match: /^Transport refused engine start: /,
    render: (_m, { t }) => t.diagTab.logHuman.transportRefused,
  },
  {
    match: /^Failed to spawn /,
    render: (_m, { t }) => t.diagTab.logHuman.spawnFailed,
  },
  {
    match: /^Error checking process status: /,
    render: (_m, { t }) => t.diagTab.logHuman.statusCheckFailed,
  },
];

/**
 * Returns a human phrase for a frequent log line, or the original
 * message unchanged when no rule matches (unknown lines, and lines
 * that are already human Russian).
 */
export function humanizeLogMessage(
  message: string,
  t: Translations,
  language: Language,
  strategies?: LuaStrategy[],
): string {
  const trimmed = message.trim();
  const ctx: HumanizeContext = {
    t,
    language,
    strategyName: (id) => strategies?.find((s) => s.id === id)?.name ?? id,
  };
  for (const rule of RULES) {
    const m = rule.match.exec(trimmed);
    if (m) return rule.render(m, ctx);
  }
  return message;
}
