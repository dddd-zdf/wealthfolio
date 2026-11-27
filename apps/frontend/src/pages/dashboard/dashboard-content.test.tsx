import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { calculatePerformanceSummary, getIntradayValuations } from "@/adapters";
import type { HistoryChartData } from "@/components/history-chart";
import { AccountsSummary } from "./accounts-summary";
import { useQuery } from "@tanstack/react-query";
import { useCurrentValuation } from "@/hooks/use-current-account-valuations";
import { useHoldings } from "@/hooks/use-holdings";
import { useValuationHistory } from "@/hooks/use-valuation-history";
import { useSettingsContext } from "@/lib/settings-provider";
import { QueryKeys } from "@/lib/query-keys";
import { DashboardContent } from "./dashboard-content";

const uiMocks = vi.hoisted(() => ({
  realIntervals: false,
  intervalCode: "3M" as "1D" | "1W" | "3M" | "ALL" | null,
  togglePrivacy: vi.fn(),
}));

vi.mock("@/adapters", () => ({
  calculatePerformanceSummary: vi.fn(),
  getIntradayValuations: vi.fn(),
  isWeb: true,
}));

vi.mock("@/hooks/use-health", () => ({
  useHealthStatus: () => ({ data: undefined, isLoading: false }),
}));
vi.mock("@/components/privacy-toggle", () => ({
  PrivacyToggle: () => <button onClick={uiMocks.togglePrivacy}>Hide Balance</button>,
}));

vi.mock("@/hooks/use-calculate-portfolio", () => ({
  useUpdatePortfolioMutation: () => ({ mutate: vi.fn(), isPending: false }),
  useRecalculatePortfolioMutation: () => ({ mutate: vi.fn(), isPending: false }),
}));

vi.mock("@/components/history-chart", () => ({
  HistoryChart: ({
    data,
    timeDomain,
    referenceValue,
    onActivePointChange,
  }: {
    data: HistoryChartData[];
    timeDomain?: [number, number];
    referenceValue?: number;
    onActivePointChange: (point: HistoryChartData | null) => void;
  }) => (
    <div
      data-testid="history-chart"
      data-time-domain={JSON.stringify(timeDomain ?? null)}
      data-chart-values={JSON.stringify(data.map((point) => point.totalValue))}
      data-reference-value={referenceValue}
      onMouseEnter={() => onActivePointChange(data[1] ?? null)}
      onMouseLeave={() => onActivePointChange(null)}
    >
      history-chart
    </div>
  ),
}));

vi.mock("@/hooks", () => ({
  useHapticFeedback: () => ({ triggerHaptic: vi.fn() }),
}));

vi.mock("@/hooks/use-holdings", () => ({
  useHoldings: vi.fn(),
}));

vi.mock("@/hooks/use-current-account-valuations", () => ({
  useCurrentValuation: vi.fn(),
}));

vi.mock("@/hooks/use-valuation-history", () => ({
  useValuationHistory: vi.fn(),
}));

vi.mock("@/lib/settings-provider", () => ({
  useSettingsContext: vi.fn(),
}));

vi.mock("@tanstack/react-query", () => ({
  keepPreviousData: Symbol("keepPreviousData"),
  useQuery: vi.fn(),
}));

vi.mock("@wealthfolio/ui", async () => {
  const { getInitialIntervalData } =
    await import("@wealthfolio/ui/components/financial/interval-selector");
  return {
    GainAmount: ({ value }: { value: number }) => <span>{`gain-amount:${value}`}</span>,
    GainPercent: ({ value }: { value: number }) => <span>{`gain-percent:${value}`}</span>,
    getInitialIntervalData: (code?: "ALL" | "3M", asOf?: Date) =>
      uiMocks.realIntervals
        ? getInitialIntervalData(code, asOf)
        : code === "ALL"
          ? {
              range: {
                from: new Date("1970-01-01T00:00:00Z"),
                to: new Date("2026-06-24T00:00:00Z"),
              },
              description: "All Time",
            }
          : {
              range: {
                from: new Date("2026-03-01T00:00:00Z"),
                to: new Date("2026-06-01T00:00:00Z"),
              },
              description: "3M",
            },
    IntervalSelector: () => <div>interval-selector</div>,
    usePersistentState: (key: string, initial: unknown) => [
      key.endsWith("dashboard-period")
        ? (JSON.parse(window.localStorage.getItem(key) ?? "null") ??
          (uiMocks.intervalCode ? { kind: "portfolio", code: uiMocks.intervalCode } : initial))
        : initial,
      vi.fn(),
    ],
    useDateFormatting: () => ({
      formatDate: () => "Jun 1, 2026",
      formatTime: () => "12:30",
      formatCalendarDate: () => "Jun 1, 2026",
    }),
  };
});

vi.mock("@wealthfolio/ui/components/ui/skeleton", () => ({
  Skeleton: () => <div>loading</div>,
}));

vi.mock("@/pages/dashboard/portfolio-update-trigger", async (importOriginal) => {
  const { PortfolioUpdateTrigger } =
    await importOriginal<typeof import("./portfolio-update-trigger")>();
  return {
    PortfolioUpdateTrigger: ({
      children,
      lastCalculatedAt,
      notices,
    }: {
      children: ReactNode;
      lastCalculatedAt?: string;
      notices?: string[];
    }) => (
      <div>
        <div data-testid="portfolio-notices">{JSON.stringify(notices ?? [])}</div>
        <div data-testid="portfolio-as-of">{lastCalculatedAt ?? ""}</div>
        <PortfolioUpdateTrigger lastCalculatedAt={lastCalculatedAt} notices={notices}>
          {children}
        </PortfolioUpdateTrigger>
      </div>
    ),
  };
});

vi.mock("@/features/spending/components/spending-date-picker", () => ({
  SpendingDatePicker: () => null,
}));

vi.mock("./accounts-summary", () => ({
  AccountsSummary: vi.fn(() => <div>accounts-summary</div>),
}));

vi.mock("./balance", () => ({
  default: ({ targetValue, isUnavailable }: { targetValue: number; isUnavailable?: boolean }) => (
    <div>{isUnavailable ? "balance:N/A" : `balance:${targetValue}`}</div>
  ),
}));

vi.mock("./goals", () => ({
  default: () => <div>saving-goals</div>,
}));

vi.mock("./top-holdings", () => ({
  default: () => <div>top-holdings</div>,
}));

const mockUseQuery = vi.mocked(useQuery);
const mockUseCurrentValuation = vi.mocked(useCurrentValuation);
const mockUseHoldings = vi.mocked(useHoldings);
const mockUseValuationHistory = vi.mocked(useValuationHistory);
const mockUseSettingsContext = vi.mocked(useSettingsContext);

describe("DashboardContent", () => {
  beforeEach(() => {
    uiMocks.intervalCode = "3M";
    uiMocks.realIntervals = false;
    vi.clearAllMocks();
  });

  function mockCurrentValuation(totalValueBase = 125) {
    mockUseCurrentValuation.mockReturnValue({
      currentValuation: {
        summary: {
          scopeId: "all",
          baseCurrency: "USD",
          cashBalanceBase: 25,
          investmentMarketValueBase: totalValueBase - 25,
          totalValueBase,
          holdingsCount: 2,
          accountCount: 1,
          currencySplit: [],
          cashCurrencySplit: [],
          sourceDataAsOf: "2026-06-01T12:30:00Z",
          calculatedAt: "2026-06-01T13:00:00Z",
          warnings: [],
        },
        accounts: [],
      },
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useCurrentValuation>);
  }

  it.each(["unavailable", "partialUnpriced"] as const)(
    "does not show an incomplete %s subtotal as an investment balance",
    (status) => {
      mockUseSettingsContext.mockReturnValue({
        settings: { baseCurrency: "AED" },
      } as unknown as ReturnType<typeof useSettingsContext>);
      mockCurrentValuation(20);
      const previous = mockUseCurrentValuation({ type: "all" });
      mockUseCurrentValuation.mockReturnValue({
        ...previous,
        currentValuation: {
          ...previous.currentValuation!,
          summary: {
            ...previous.currentValuation!.summary,
            valueStatus: status,
            missingFx: [],
          },
        },
      });
      mockUseHoldings.mockReturnValue({
        holdings: [],
        isLoading: false,
        error: null,
      } as unknown as ReturnType<typeof useHoldings>);
      mockUseValuationHistory.mockReturnValue({
        valuationHistory: [],
        isLoading: false,
        error: null,
      } as unknown as ReturnType<typeof useValuationHistory>);
      mockUseQuery.mockReturnValue({ isLoading: false, data: null } as unknown as ReturnType<
        typeof useQuery
      >);
      render(<DashboardContent />);
      expect(screen.getByText("balance:N/A")).toBeInTheDocument();
      expect(screen.queryByText("balance:20")).not.toBeInTheDocument();
      expect(
        screen.queryByText("Total unavailable: some prices or exchange rates are missing."),
      ).not.toBeInTheDocument();
    },
  );

  it("keeps history gaps visible without reporting growth across an incomplete period", () => {
    mockCurrentValuation(125);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        { valuationDate: "2026-05-30", totalValueBase: 100, valueStatus: "complete" },
        { valuationDate: "2026-05-31", totalValueBase: 0, valueStatus: "unavailable" },
        { valuationDate: "2026-06-01", totalValueBase: 125, valueStatus: "complete" },
      ],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: {
        summary: { amountStatus: "complete", percentStatus: "complete", amount: 25, percent: 0.25 },
      },
    } as unknown as ReturnType<typeof useQuery>);
    render(<DashboardContent />);
    expect(screen.getByText("balance:125")).toBeInTheDocument();
    expect(screen.queryByText("gain-amount:25")).not.toBeInTheDocument();
    expect(screen.queryByText("gain-percent:0.25")).not.toBeInTheDocument();
    expect(
      screen.queryByText(
        "Some historical values are unavailable. Changes cannot be calculated for this period.",
      ),
    ).not.toBeInTheDocument();
  });

  it.each([
    ["past custom range", "2026-06-01", true],
    ["range including today", "2026-06-02", false],
  ] as const)("uses period completeness for a %s", (_label, endDate, showsReturn) => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-06-02T12:00:00Z"));
    window.localStorage.setItem(
      "dashboard-period",
      JSON.stringify({
        kind: "range",
        from: "2026-05-01",
        to: endDate,
      }),
    );
    try {
      mockUseSettingsContext.mockReturnValue({
        settings: { baseCurrency: "USD", timezone: "UTC" },
      } as ReturnType<typeof useSettingsContext>);
      mockCurrentValuation();
      const current = mockUseCurrentValuation({ type: "all" });
      mockUseCurrentValuation.mockReturnValue({
        ...current,
        currentValuation: {
          ...current.currentValuation!,
          summary: { ...current.currentValuation!.summary, valueStatus: "unavailable" },
        },
      });
      mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
        typeof useHoldings
      >);
      mockUseValuationHistory.mockReturnValue({
        valuationHistory: [
          { valuationDate: endDate, totalValueBase: 125, valueStatus: "complete" },
        ],
        isLoading: false,
      } as ReturnType<typeof useValuationHistory>);
      mockUseQuery.mockReturnValue({
        isLoading: false,
        data: {
          period: { startDate: "2026-05-01", endDate },
          summary: {
            amountStatus: "complete",
            percentStatus: "complete",
            amount: 25,
            percent: 0.25,
          },
        },
      } as unknown as ReturnType<typeof useQuery>);
      render(<DashboardContent />);
      expect(Boolean(screen.queryByText("gain-amount:25"))).toBe(showsReturn);
      expect(Boolean(screen.queryByText("gain-percent:0.25"))).toBe(showsReturn);
    } finally {
      window.localStorage.removeItem("dashboard-period");
      vi.useRealTimers();
    }
  });

  it("does not pass backend performance warnings to dashboard header notices", () => {
    mockCurrentValuation(125);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        {
          valuationDate: "2026-06-01",
          totalValueBase: 1000,
          netContributionBase: 900,
          baseCurrency: "USD",
          calculatedAt: "2026-06-01T12:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: {
        scope: { id: "portfolio:all", currency: "USD" },
        period: { startDate: "2026-03-01", endDate: "2026-06-01" },
        mode: "timeWeighted",
        returns: {
          twr: 0.1,
          annualizedTwr: null,
          irr: null,
          annualizedIrr: null,
          valueReturn: 0.1,
        },
        attribution: {
          contributions: 0,
          distributions: 0,
          income: 0,
          realizedPnl: 0,
          unrealizedPnlChange: 100,
          fxEffect: 0,
          fees: 0,
          taxes: 0,
          residual: 0,
        },
        risk: {
          volatility: null,
          maxDrawdown: null,
          peakDate: null,
          troughDate: null,
          recoveryDate: null,
          drawdownDurationDays: null,
        },
        dataQuality: {
          status: "partial",
          warnings: ["Backend performance warning that belongs in Health Center."],
          notApplicableReasons: [],
        },
        series: [],
      },
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);

    expect(screen.getByTestId("portfolio-notices")).toHaveTextContent("[]");
    expect(screen.queryByText(/backend performance warning/i)).not.toBeInTheDocument();
  });

  it("uses scoped current valuation for the dashboard headline", () => {
    mockCurrentValuation(125);
    mockUseHoldings.mockReturnValue({
      holdings: [
        {
          holdingType: "security",
          marketValue: { base: 100, local: 100 },
        },
      ],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: null,
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);

    expect(screen.getByText("balance:125")).toBeInTheDocument();
    expect(screen.queryByText("balance:100")).not.toBeInTheDocument();
    expect(screen.getByTestId("portfolio-as-of")).toHaveTextContent("2026-06-01T12:30:00Z");
    expect(screen.getByTestId("portfolio-as-of")).not.toHaveTextContent("2026-06-01T13:00:00Z");
  });

  it("shows the hovered point and its deposit-adjusted gain until the pointer leaves", async () => {
    mockCurrentValuation(160);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        { valuationDate: "2026-05-01", totalValueBase: 100, netContributionBase: 80 },
        { valuationDate: "2026-05-02", totalValueBase: 130, netContributionBase: 100 },
        { valuationDate: "2026-05-03", totalValueBase: 160, netContributionBase: 100 },
      ],
      isLoading: false,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: {
        period: { startDate: "2026-05-01", endDate: "2026-05-03" },
        summary: { amount: 40, amountStatus: "complete", percent: 0.4, percentStatus: "complete" },
      },
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);
    const user = userEvent.setup();
    const chart = screen.getByTestId("history-chart");
    expect(chart).toHaveAttribute("data-reference-value", "100");
    await user.hover(chart);
    expect(screen.getByText("balance:130")).toBeInTheDocument();
    expect(screen.getByText("gain-amount:10")).toBeInTheDocument();
    expect(screen.getByText("gain-percent:0.1")).toBeInTheDocument();

    await user.unhover(chart);
    expect(screen.getByText("balance:160")).toBeInTheDocument();
    expect(screen.getByText("gain-amount:40")).toBeInTheDocument();
    expect(screen.getByText("gain-percent:0.4")).toBeInTheDocument();
  });

  it("uses the performance summary on its final daily point", async () => {
    mockCurrentValuation(130);
    mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
      typeof useHoldings
    >);
    mockUseSettingsContext.mockReturnValue({ settings: { baseCurrency: "USD" } } as ReturnType<
      typeof useSettingsContext
    >);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        { valuationDate: "2026-05-01", totalValueBase: 100, netContributionBase: 80 },
        { valuationDate: "2026-05-02", totalValueBase: 130, netContributionBase: 100 },
      ],
      isLoading: false,
    } as ReturnType<typeof useValuationHistory>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: {
        period: { startDate: "2026-05-01", endDate: "2026-05-02" },
        summary: { amount: 30, amountStatus: "complete", percent: 0.25, percentStatus: "complete" },
      },
    } as ReturnType<typeof useQuery>);

    render(<DashboardContent />);
    await userEvent.hover(screen.getByTestId("history-chart"));
    expect(screen.getByText("gain-amount:30")).toBeInTheDocument();
    expect(screen.getByText("gain-percent:0.25")).toBeInTheDocument();
  });

  it("uses the 1W period opening value when intraday observations begin later", async () => {
    uiMocks.intervalCode = "1W";
    mockCurrentValuation(170);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        { valuationDate: "2026-05-01", totalValueBase: 100, netContributionBase: 90 },
        { valuationDate: "2026-05-06", totalValueBase: 170, netContributionBase: 110 },
      ],
      isLoading: false,
    } as ReturnType<typeof useValuationHistory>);
    const intradayPoints = [
      {
        timestamp: "2026-05-04T14:00:00Z",
        totalValueBase: 140,
        netContributionBase: 110,
      },
      {
        timestamp: "2026-05-05T14:00:00Z",
        totalValueBase: 160,
        netContributionBase: 110,
      },
    ];
    mockUseQuery.mockImplementation(
      (options) =>
        ({
          isLoading: false,
          data:
            options.queryKey[1] === "intraday"
              ? intradayPoints
              : {
                  period: { startDate: "2026-05-01", endDate: "2026-05-06" },
                  summary: {
                    amount: 50,
                    amountStatus: "complete",
                    percent: 0.5,
                    percentStatus: "complete",
                  },
                },
        }) as ReturnType<typeof useQuery>,
    );

    render(<DashboardContent />);
    const chart = screen.getByTestId("history-chart");
    expect(chart).toHaveAttribute("data-reference-value", "100");
    await userEvent.hover(chart);
    expect(screen.getByText("gain-amount:40")).toBeInTheDocument();
    expect(screen.getByText("gain-percent:0.4")).toBeInTheDocument();
  });

  it("waits for the 1D baseline before showing hover gains", async () => {
    uiMocks.intervalCode = "1D";
    mockCurrentValuation(1130);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: true,
      isComplete: false,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({ valuationHistory: [], isLoading: false });
    const intradayPoints = [
      {
        timestamp: "2026-10-02T13:30:00Z",
        totalValueBase: 1020,
        netContributionBase: 900,
      },
      {
        timestamp: "2026-10-02T16:45:00Z",
        totalValueBase: 1125,
        netContributionBase: 1000,
      },
    ];
    mockUseQuery.mockImplementation(
      (options) =>
        ({
          isLoading: false,
          data: options.queryKey[1] === "intraday" ? intradayPoints : undefined,
        }) as ReturnType<typeof useQuery>,
    );

    const { rerender } = render(<DashboardContent />);
    const chart = screen.getByTestId("history-chart");
    await userEvent.hover(chart);
    expect(screen.getByText("balance:1125")).toBeInTheDocument();
    expect(screen.queryByText(/^gain-amount:/)).toBeNull();
    expect(chart).not.toHaveAttribute("data-reference-value");

    mockUseHoldings.mockReturnValue({
      holdings: [{ holdingType: "security", dayChange: { local: 30, base: 30 } }],
      isLoading: false,
      isComplete: true,
    } as unknown as ReturnType<typeof useHoldings>);
    rerender(<DashboardContent />);
    expect(chart).toHaveAttribute("data-reference-value", "1000");
    expect(screen.getByText("gain-amount:25")).toBeInTheDocument();
    expect(screen.getByText(`gain-percent:${25 / 1100}`)).toBeInTheDocument();
  });

  it("sums holdings' last-session moves for 1D instead of the flat weekend period", () => {
    uiMocks.intervalCode = "1D";
    mockCurrentValuation(1030);
    mockUseHoldings.mockReturnValue({
      holdings: [
        { holdingType: "security", dayChange: { local: 20, base: 20 } },
        { holdingType: "security", dayChange: { local: 10, base: 10 } },
      ],
      isLoading: false,
      isComplete: true,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    // Friday's session, half done: 9:30 to 12:45 ET.
    const intradayPoints = [
      { timestamp: "2026-10-02T13:30:00Z", totalValueBase: 1000, netContributionBase: 900 },
      { timestamp: "2026-10-02T16:45:00Z", totalValueBase: 1030, netContributionBase: 900 },
    ];
    mockUseQuery.mockImplementation(
      (options) =>
        ({
          isLoading: false,
          isPending: false,
          // The period summary for a weekend would be flat; it must not be used.
          data:
            (options.queryKey as unknown[])[1] === "intraday"
              ? intradayPoints
              : {
                  summary: {
                    amount: 0,
                    amountStatus: "complete",
                    percent: 0,
                    percentStatus: "complete",
                  },
                },
        }) as unknown as ReturnType<typeof useQuery>,
    );

    const { rerender } = render(<DashboardContent />);

    expect(screen.getByText("gain-amount:30")).toBeInTheDocument();
    expect(screen.getByText("gain-percent:0.03")).toBeInTheDocument();
    const performanceQuery = mockUseQuery.mock.calls.find(
      ([options]) => (options.queryKey as unknown[])[0] === QueryKeys.PERFORMANCE_SUMMARY,
    );
    expect(performanceQuery?.[0].enabled).toBe(false);
    // The axis spans the full 6.5-hour session, so the line stops halfway.
    const open = Date.parse("2026-10-02T13:30:00Z");
    expect(screen.getByTestId("history-chart")).toHaveAttribute(
      "data-time-domain",
      JSON.stringify([open, open + 6.5 * 60 * 60 * 1000]),
    );
    expect(screen.getByTestId("history-chart")).toHaveAttribute("data-reference-value", "1000");

    const completeHoldings = mockUseHoldings.mock.results.at(-1)?.value;
    mockUseHoldings.mockReturnValue({
      ...completeHoldings,
      isComplete: false,
      incompleteAccounts: [
        { accountId: "pending", status: "pending", reasonCode: "PROJECTION_PENDING" },
      ],
    });
    rerender(<DashboardContent />);
    expect(screen.queryByText("gain-amount:30")).toBeNull();
    expect(screen.queryByText("gain-percent:0.03")).toBeNull();
  });

  it("requests all-time valuation history without the 1970 sentinel range", () => {
    uiMocks.intervalCode = "ALL";
    mockCurrentValuation(125);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        { valuationDate: "2026-05-01", totalValueBase: 100, netContributionBase: 80 },
        { valuationDate: "2026-05-02", totalValueBase: 125, netContributionBase: 80 },
      ],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: null,
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);
    expect(screen.getByTestId("history-chart")).not.toHaveAttribute("data-reference-value");

    expect(mockUseValuationHistory).toHaveBeenCalledWith(
      undefined,
      { type: "all" },
      { keepPrevious: false },
    );
  });

  it("starts performance queries immediately when the chart has cached history", () => {
    mockCurrentValuation(125);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        {
          valuationDate: "2026-06-01",
          totalValueBase: 1000,
          netContributionBase: 900,
          baseCurrency: "USD",
          calculatedAt: "2026-06-01T12:00:00Z",
        },
      ],
      isLoading: true,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: null,
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);

    const performanceQueryOptions = mockUseQuery.mock.calls.find(
      ([options]) => options.queryKey[0] === QueryKeys.PERFORMANCE_SUMMARY,
    )?.[0];
    expect(performanceQueryOptions?.enabled).toBe(true);
    expect(performanceQueryOptions?.placeholderData).toBeUndefined();
    expect(screen.getByText("accounts-summary")).toBeInTheDocument();
  });

  it("does not render a failed current valuation as zero", () => {
    mockUseCurrentValuation.mockReturnValue({
      currentValuation: undefined,
      isLoading: false,
      error: new Error("current valuation failed"),
    } as unknown as ReturnType<typeof useCurrentValuation>);
    mockUseHoldings.mockReturnValue({
      holdings: [],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useHoldings>);
    mockUseValuationHistory.mockReturnValue({
      valuationHistory: [
        {
          valuationDate: "2026-06-01",
          totalValueBase: 1000,
          netContributionBase: 900,
          baseCurrency: "USD",
          calculatedAt: "2026-06-01T12:00:00Z",
        },
      ],
      isLoading: false,
      error: null,
    } as unknown as ReturnType<typeof useValuationHistory>);
    mockUseSettingsContext.mockReturnValue({
      settings: { baseCurrency: "USD" },
    } as unknown as ReturnType<typeof useSettingsContext>);
    mockUseQuery.mockReturnValue({
      isLoading: false,
      data: null,
    } as unknown as ReturnType<typeof useQuery>);

    render(<DashboardContent />);

    expect(screen.getByText("balance:N/A")).toBeInTheDocument();
    expect(screen.queryByText("balance:0")).not.toBeInTheDocument();
    expect(screen.getByTestId("portfolio-as-of")).toHaveTextContent("");
  });
  it("anchors the default 1D preset to the configured day", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-12-31T16:30:00Z"));
      uiMocks.realIntervals = true;
      uiMocks.intervalCode = null;
      mockCurrentValuation();
      mockUseSettingsContext.mockReturnValue({
        settings: { baseCurrency: "USD", timezone: "Asia/Shanghai" },
      } as ReturnType<typeof useSettingsContext>);
      mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
        typeof useHoldings
      >);
      mockUseValuationHistory.mockReturnValue({ valuationHistory: [], isLoading: false });
      mockUseQuery.mockReturnValue({ data: undefined, isLoading: false } as ReturnType<
        typeof useQuery
      >);
      render(<DashboardContent />);
      expect(mockUseValuationHistory).toHaveBeenLastCalledWith(
        {
          from: new Date(2026, 11, 31),
          to: new Date(2027, 0, 1),
        },
        { type: "all" },
        { keepPrevious: false },
      );
    } finally {
      vi.useRealTimers();
    }
  });
  it.each(["3M", "1D"] as const)(
    "uses one portfolio scope for all dashboard data in %s",
    async (interval) => {
      uiMocks.intervalCode = interval;
      mockCurrentValuation();
      mockUseSettingsContext.mockReturnValue({ settings: { baseCurrency: "USD" } } as ReturnType<
        typeof useSettingsContext
      >);
      mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
        typeof useHoldings
      >);
      mockUseValuationHistory.mockReturnValue({ valuationHistory: [], isLoading: false });
      mockUseQuery.mockReturnValue({ data: null, isLoading: false } as ReturnType<typeof useQuery>);
      const scope = { type: "portfolio", portfolioId: "retirement" } as const;
      render(<DashboardContent scope={scope} scopeLabel="Retirement" accountIds={["account-1"]} />);
      expect(mockUseHoldings).toHaveBeenCalledWith(scope, { enabled: true });
      expect(mockUseCurrentValuation).toHaveBeenCalledWith(scope, { includeAccounts: true });
      expect(mockUseValuationHistory.mock.calls.at(-1)?.[1]).toEqual(scope);
      expect(vi.mocked(AccountsSummary).mock.calls.at(-1)?.[0]).toMatchObject({
        accountIds: ["account-1"],
      });
      const query = mockUseQuery.mock.calls.find(
        ([options]) => options.queryKey[0] === QueryKeys.PERFORMANCE_SUMMARY,
      )?.[0];
      expect(query?.queryKey).toContain("portfolio:retirement");
      await (query?.queryFn as () => Promise<unknown>)();
      expect(calculatePerformanceSummary).toHaveBeenCalledWith(
        expect.objectContaining({ itemId: "portfolio:retirement", filter: scope }),
      );
      const intraday = mockUseQuery.mock.calls.find(
        ([options]) => options.queryKey[1] === "intraday",
      )?.[0];
      expect(intraday?.queryKey).toContain("portfolio:retirement");
      expect(intraday?.queryKey).toContain("USD");
      if (interval === "1D") {
        await (intraday?.queryFn as () => Promise<unknown>)();
        expect(getIntradayValuations).toHaveBeenCalledWith(scope, "1D");
      }
      expect(screen.getByText("Retirement")).toBeInTheDocument();
    },
  );

  it.each(["1D", "3M"] as const)("prioritizes the cold %s chart for a new scope", (interval) => {
    uiMocks.intervalCode = interval;
    mockCurrentValuation();
    mockUseSettingsContext.mockReturnValue({ settings: { baseCurrency: "USD" } } as ReturnType<
      typeof useSettingsContext
    >);
    mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
      typeof useHoldings
    >);
    const scope = { type: "portfolio", portfolioId: "next" } as const;
    const view = <DashboardContent scope={scope} />;
    const performanceQuery = () =>
      mockUseQuery.mock.calls
        .filter(([options]) => options.queryKey[0] === QueryKeys.PERFORMANCE_SUMMARY)
        .at(-1)?.[0];

    mockUseValuationHistory.mockReturnValue({ valuationHistory: undefined, isLoading: true });
    mockUseQuery.mockImplementation(
      (options) =>
        ({
          isLoading: options.queryKey[1] === "intraday" && interval === "1D",
          data: undefined,
        }) as ReturnType<typeof useQuery>,
    );
    const { rerender } = render(view);
    expect(screen.getByTestId("history-chart")).toHaveAttribute("data-chart-values", "[]");
    expect(mockUseCurrentValuation).toHaveBeenLastCalledWith(scope, { includeAccounts: true });
    expect(mockUseHoldings).toHaveBeenLastCalledWith(scope, { enabled: false });
    expect(performanceQuery()?.enabled).toBe(false);
    expect(vi.mocked(AccountsSummary).mock.calls.at(-1)?.[0].loadDetails).toBe(false);

    if (interval === "1D") {
      // Recorded points can paint while the daily fallback is still pending.
      mockUseQuery.mockImplementation(
        (options) =>
          ({
            isLoading: false,
            data:
              options.queryKey[1] === "intraday"
                ? [
                    {
                      timestamp: "2026-06-01T13:30:00Z",
                      totalValueBase: 450,
                      netContributionBase: 0,
                    },
                  ]
                : undefined,
          }) as ReturnType<typeof useQuery>,
      );
    } else {
      mockUseValuationHistory.mockReturnValue({
        valuationHistory: [{ valuationDate: "2026-06-01", totalValueBase: 450 }],
        isLoading: false,
      } as ReturnType<typeof useValuationHistory>);
    }
    rerender(<DashboardContent scope={scope} />);
    expect(screen.getByTestId("history-chart")).toHaveAttribute("data-chart-values", "[450]");
    expect(mockUseHoldings).toHaveBeenLastCalledWith(scope, { enabled: true });
    expect(performanceQuery()?.enabled).toBe(interval !== "1D");
    expect(vi.mocked(AccountsSummary).mock.calls.at(-1)?.[0].loadDetails).toBe(true);
  });

  it("keeps privacy keyboard focus and activation separate from the update hover card", async () => {
    mockCurrentValuation();
    mockUseSettingsContext.mockReturnValue({ settings: { baseCurrency: "USD" } } as ReturnType<
      typeof useSettingsContext
    >);
    mockUseHoldings.mockReturnValue({ holdings: [], isLoading: false } as unknown as ReturnType<
      typeof useHoldings
    >);
    mockUseValuationHistory.mockReturnValue({ valuationHistory: [], isLoading: false });
    mockUseQuery.mockReturnValue({ data: null, isLoading: false } as ReturnType<typeof useQuery>);
    render(<DashboardContent scopeLabel="All Accounts" />);
    expect(screen.queryByText("All Accounts")).not.toBeInTheDocument();

    const privacy = screen.getByRole("button", { name: "Hide Balance" });
    expect(privacy.closest("a")).toBeNull();
    // Header navigation comes before the balance controls in keyboard order.
    for (let tabs = 0; tabs < 3 && document.activeElement !== privacy; tabs++) {
      await userEvent.tab();
    }
    expect(privacy).toHaveFocus();
    await new Promise((resolve) => setTimeout(resolve, 800));
    expect(screen.queryByRole("button", { name: "Update prices" })).not.toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(uiMocks.togglePrivacy).toHaveBeenCalledOnce();
    expect(screen.getByText("balance:125")).toBeInTheDocument();
  });
});
