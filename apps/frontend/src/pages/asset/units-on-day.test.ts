import { describe, expect, it } from "vitest";
import type { ActivityDetails } from "@/lib/types";
import { unitsOnDay } from "./units-on-day";

const activity = (activityType: string, date: string, quantity: string, status = "POSTED") =>
  ({ activityType, date: `${date}T12:00:00Z`, quantity, status }) as unknown as ActivityDetails;

describe("unitsOnDay", () => {
  const activities = [
    activity("BUY", "2026-09-18", "9"),
    activity("BUY", "2026-09-04", "9"),
    activity("SELL", "2026-09-10", "2"),
    activity("DEPOSIT", "2026-09-18", "100"),
    activity("BUY", "2026-09-20", "5", "DRAFT"),
  ];

  it("keeps current units when nothing happened after the day", () => {
    expect(unitsOnDay(100, activities, "2026-09-18")).toBe(100);
  });

  it("removes later posted trades", () => {
    expect(unitsOnDay(100, activities, "2026-09-17")).toBe(91);
    expect(unitsOnDay(100, activities, "2026-09-05")).toBe(93);
    expect(unitsOnDay(100, activities, "2026-09-01")).toBe(84);
  });
});
