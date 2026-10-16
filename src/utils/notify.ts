import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

/**
 * Module-level kill-switch, synced from backend `AppSettings.notifications_enabled`
 * at startup and whenever the user toggles it in Settings.
 */
let notificationsEnabled = true;

export function setNotificationsEnabled(enabled: boolean): void {
  notificationsEnabled = enabled;
}

/**
 * Shows a Windows toast notification for a key engine event.
 * Best-effort: never throws and never blocks the UI flow.
 */
export async function notifyUser(title: string, body: string): Promise<void> {
  if (!notificationsEnabled) return;
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (!granted) return;
    await sendNotification({ title, body });
  } catch {
    // Ignore: a missing notification must not break the app.
  }
}
