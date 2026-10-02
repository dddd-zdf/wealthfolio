import { ActivityStatus, ActivityType } from "@/lib/constants";
import type { ActivityDetails } from "@/lib/types";

const UNIT_CHANGES: Partial<Record<ActivityType, 1 | -1>> = {
  [ActivityType.BUY]: 1,
  [ActivityType.TRANSFER_IN]: 1,
  [ActivityType.SELL]: -1,
  [ActivityType.TRANSFER_OUT]: -1,
};

/** Units held at the end of `day` (yyyy-MM-dd): today's units minus later trades. */
export function unitsOnDay(currentUnits: number, activities: ActivityDetails[], day: string) {
  return activities.reduce((units, activity) => {
    const sign = UNIT_CHANGES[activity.activityType];
    const activityDay = new Date(activity.date).toISOString().slice(0, 10);
    if (!sign || activity.status !== ActivityStatus.POSTED || activityDay <= day) return units;
    return units - sign * Number(activity.quantity ?? 0);
  }, currentUnits);
}
