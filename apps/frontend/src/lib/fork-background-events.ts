/**
 * Fork: scheduled server runs keep data fresh silently. Their events carry
 * `background: true`; listeners refresh data but show no notifications.
 */
export function isBackgroundEvent(payload: unknown): boolean {
  return (
    typeof payload === "object" &&
    payload !== null &&
    (payload as { background?: unknown }).background === true
  );
}
