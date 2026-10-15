export function formatErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error && err.message) return err.message;
  if (typeof err === "object" && err !== null) {
    if ("message" in err && typeof (err as any).message === "string" && (err as any).message) {
      return (err as any).message;
    }
    try {
      const str = JSON.stringify(err);
      if (str && str !== "{}") return str;
    } catch {
      // ignore serialization error
    }
  }
  return String(err || "Неизвестная ошибка выполнения");
}
