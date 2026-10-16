import { render, renderHook, screen } from "@/test/render";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useActivitySearch } from "@/pages/activity/hooks/use-activity-search";
import type { NavLink } from "./app-navigation";
import { NavIcon, useNeedsReviewBadge } from "./fork-nav-badge";

vi.mock("@/pages/activity/hooks/use-activity-search", () => ({
  useActivitySearch: vi.fn(),
}));

const mockSearch = vi.mocked(useActivitySearch);

const items = [
  { title: "Dashboard", href: "/dashboard" },
  { title: "Activities", href: "/activities" },
] as NavLink[];

describe("useNeedsReviewBadge", () => {
  beforeEach(() => {
    mockSearch.mockReset();
  });

  it("badges only the Activities link with the pending count", () => {
    mockSearch.mockReturnValue({ totalRowCount: 3 } as never);
    const { result } = renderHook(() => useNeedsReviewBadge(items));

    expect(result.current[0]).toBe(items[0]);
    expect(result.current[1].badge).toBe(3);
    expect(result.current[1].badgeLabel).toBeTruthy();
    expect(mockSearch).toHaveBeenCalledWith(
      expect.objectContaining({ filters: { activityTypes: [], status: "pending" } }),
    );
  });

  it("leaves the links alone when nothing awaits review", () => {
    mockSearch.mockReturnValue({ totalRowCount: 0 } as never);
    const { result } = renderHook(() => useNeedsReviewBadge(items));

    expect(result.current).toEqual(items);
  });
});

describe("NavIcon", () => {
  it("caps the badge at 99+", () => {
    render(<NavIcon item={{ ...items[1], badge: 120 }} icon={<svg />} />);
    expect(screen.getByText("99+")).toBeInTheDocument();
  });

  it("shows no badge without a count", () => {
    const { container } = render(<NavIcon item={items[1]} icon={<svg />} />);
    expect(container.textContent).toBe("");
  });
});
