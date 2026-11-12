import { REPORTS_PERIODS, type ReportsPeriod } from "./reports-period";

export const INSIGHTS_PERIOD_STORAGE_KEY = "spending-insights-period";
/** The Insights page's complete selection, including historical range metadata. */
export const INSIGHTS_PICK_STORAGE_KEY = "spending-insights-pick";

export function normalizeReportsPeriod(value: string | null | undefined): ReportsPeriod | null {
  if (!value) return null;
  if (REPORTS_PERIODS.includes(value as ReportsPeriod)) return value as ReportsPeriod;
  return null;
}
