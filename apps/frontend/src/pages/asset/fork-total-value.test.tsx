import { render, renderHook } from "@/test/render";
import { describe, expect, it, vi } from "vitest";

import { useTotalValueEntry } from "./fork-total-value";

let savePrice: ((date: Date, price: number) => void) | undefined;

vi.mock("./update-total-value-dialog", () => ({
  UpdateTotalValueDialog: ({ onSave }: { onSave: (date: Date, price: number) => void }) => {
    savePrice = onSave;
    return null;
  },
}));

type EntryOptions = Parameters<typeof useTotalValueEntry>[0];

function options(overrides: Partial<EntryOptions>): EntryOptions {
  return {
    isManualDataSource: true,
    unitsOn: () => 4,
    initialEntries: [],
    onSaveQuote: vi.fn(async () => undefined),
    currency: "CAD",
    assetId: "fund",
    ...overrides,
  };
}

describe("useTotalValueEntry", () => {
  it("offers the action only for manual prices with known units", () => {
    expect(
      renderHook(() => useTotalValueEntry(options({}))).result.current.onUpdateValue,
    ).toBeTypeOf("function");
    expect(
      renderHook(() => useTotalValueEntry(options({ isManualDataSource: false }))).result.current
        .onUpdateValue,
    ).toBeUndefined();
    expect(
      renderHook(() => useTotalValueEntry(options({ unitsOn: undefined }))).result.current
        .totalValueDialog,
    ).toBeNull();
  });

  it("saves the derived price as a manual quote, replacing that day's existing entry", () => {
    const onSaveQuote = vi.fn(async () => undefined);
    const day = new Date(2026, 9, 15);
    const existing = {
      id: "fund_2026-10-15_MANUAL",
      date: day,
      open: 10,
      high: 10,
      low: 10,
      close: 10,
      volume: 0,
      currency: "CAD",
    };
    const { result } = renderHook(() =>
      useTotalValueEntry(options({ onSaveQuote, initialEntries: [existing] })),
    );
    render(<>{result.current.totalValueDialog}</>);

    savePrice?.(day, 12.5);

    expect(onSaveQuote).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "fund_2026-10-15_MANUAL",
        dataSource: "MANUAL",
        assetId: "fund",
        close: 12.5,
        open: 12.5,
        currency: "CAD",
      }),
    );
  });
});
