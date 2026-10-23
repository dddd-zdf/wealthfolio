// Fork: the Activities nav item shows how many activities await review.
import { useActivitySearch } from "@/pages/activity/hooks/use-activity-search";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { NavLink } from "./app-navigation";

/** Copies `items`, badging the Activities link with the pending-review count. */
export function useNeedsReviewBadge(items: NavLink[]): NavLink[] {
  const { t } = useTranslation();
  const { totalRowCount: needsReviewCount } = useActivitySearch({
    mode: "paginated",
    filters: { activityTypes: [], status: "pending" },
    searchQuery: "",
    sorting: [],
    pageIndex: 0,
    pageSize: 1,
  });
  return items.map((item) =>
    item.href === "/activities" && needsReviewCount > 0
      ? {
          ...item,
          badge: needsReviewCount,
          badgeLabel: t("activity:needs_review_alert.title", { count: needsReviewCount }),
        }
      : item,
  );
}

/** A nav icon with the item's badge, if it has one. */
export function NavIcon({ item, icon }: { item: NavLink; icon: ReactNode }) {
  return (
    <span aria-hidden="true" className="relative">
      {icon}
      {item.badge ? (
        <span className="bg-warning text-warning-foreground absolute -end-2 -top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-2xs font-semibold leading-none">
          {item.badge > 99 ? "99+" : item.badge}
        </span>
      ) : null}
    </span>
  );
}
