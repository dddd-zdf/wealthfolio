import { render, screen } from "@/test/render";
import { describe, expect, it, vi } from "vitest";

import { SpendingTotal } from "./fork-spending-total";

vi.mock("@/pages/dashboard/balance", () => ({
  default: ({ targetValue }: { targetValue: number }) => <span>total {targetValue}</span>,
}));

vi.mock("@wealthfolio/ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@wealthfolio/ui")>()),
  PrivacyAmount: ({ value }: { value: number }) => <span>avg {value.toFixed(2)}</span>,
}));

describe("SpendingTotal", () => {
  it("shows the monthly average for ranges longer than a month", () => {
    render(
      <SpendingTotal
        isLoading={false}
        totalSpending={3652.5}
        currency="CAD"
        dateRange={{ from: new Date(2026, 0, 1), to: new Date(2026, 11, 31) }}
      />,
    );
    // 365 days is 365 / (365.25 / 12) months.
    const expected = 3652.5 / (365 / (365.25 / 12));
    expect(screen.getByText(`avg ${expected.toFixed(2)}`)).toBeInTheDocument();
  });

  it("skips the average for a month or less", () => {
    render(
      <SpendingTotal
        isLoading={false}
        totalSpending={500}
        currency="CAD"
        dateRange={{ from: new Date(2026, 9, 1), to: new Date(2026, 9, 31) }}
      />,
    );
    expect(screen.getByText("total 500")).toBeInTheDocument();
    expect(screen.queryByText(/^avg /)).not.toBeInTheDocument();
  });
});
