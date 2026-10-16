import { render, screen } from "@/test/render";
import { YearPresets } from "@wealthfolio/ui/components/common/fork-year-presets";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

describe("YearPresets", () => {
  it("offers the current year to date, then whole earlier years", async () => {
    const onPick = vi.fn();
    const asOf = new Date(2026, 9, 16);
    render(<YearPresets count={3} asOf={asOf} selected={undefined} onPick={onPick} />);

    expect(screen.getAllByRole("button").map((button) => button.textContent)).toEqual([
      "2026",
      "2025",
      "2024",
    ]);

    await userEvent.click(screen.getByRole("button", { name: "2026" }));
    expect(onPick).toHaveBeenLastCalledWith({ from: new Date(2026, 0, 1), to: asOf });

    await userEvent.click(screen.getByRole("button", { name: "2025" }));
    expect(onPick).toHaveBeenLastCalledWith({
      from: new Date(2025, 0, 1),
      to: new Date(2025, 11, 31),
    });
  });
});
